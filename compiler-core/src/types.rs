use crate::{
    ast::{Ast, BinaryOp, Block, Expr, ExprId, Stmt, UnaryOp},
    CompileError, ErrorCode, Span,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Type {
    Unknown,
    I64,
    Bool,
}

#[derive(Clone, Copy)]
struct Local<'s> {
    name: &'s str,
    ty: Type,
    mutable: bool,
}
pub fn check(ast: &mut Ast<'_>) -> Result<(), CompileError> {
    let main = ast.function("main").ok_or_else(|| {
        CompileError::new(
            ErrorCode::MissingMain,
            Span::default(),
            "потрібна функція main",
        )
    })?;
    if main.param_count != 0 || main.return_type != Type::I64 {
        return Err(CompileError::new(
            ErrorCode::InvalidMain,
            main.span,
            "main має мати тип fn main() -> i64",
        ));
    }
    let functions: [Option<crate::Function<'_>>; crate::ast::MAX_FUNCTIONS] = {
        let mut copy = [None; crate::ast::MAX_FUNCTIONS];
        for (slot, function) in copy.iter_mut().zip(ast.functions()) {
            *slot = Some(*function);
        }
        copy
    };
    for function in functions.iter().flatten() {
        let mut locals: [Option<Local<'_>>; 128] = [None; 128];
        let mut len = 0;
        for parameter in &function.params[..function.param_count as usize] {
            insert(
                &mut locals,
                &mut len,
                Local {
                    name: parameter.name,
                    ty: parameter.ty,
                    mutable: false,
                },
                parameter.span,
            )?;
        }
        check_block(
            ast,
            &functions,
            &function.body,
            function.return_type,
            &mut locals,
            &mut len,
        )?;
    }
    Ok(())
}
fn check_block<'s>(
    ast: &mut Ast<'s>,
    functions: &[Option<crate::Function<'s>>],
    block: &Block,
    return_type: Type,
    locals: &mut [Option<Local<'s>>; 128],
    len: &mut usize,
) -> Result<(), CompileError> {
    let scope = *len;
    for id in block.ids() {
        let span = ast.stmt_span(*id);
        match ast.stmt(*id) {
            Stmt::Let {
                name,
                mutable,
                ty,
                value,
            } => {
                expect(ast, functions, value, locals, *len, ty)?;
                insert(locals, len, Local { name, ty, mutable }, span)?;
            }
            Stmt::Assign { name, value } => {
                let local = find(locals, *len, name).ok_or_else(|| {
                    CompileError::new(ErrorCode::UnknownName, span, "невідоме ім'я")
                })?;
                if !local.mutable {
                    return Err(CompileError::new(
                        ErrorCode::Unsupported,
                        span,
                        "змінна не оголошена як mut",
                    ));
                }
                expect(ast, functions, value, locals, *len, local.ty)?;
            }
            Stmt::Expr(value) => {
                expression(ast, functions, value, locals, *len)?;
            }
            Stmt::Return(value) => expect(ast, functions, value, locals, *len, return_type)?,
            Stmt::If {
                condition,
                then_block,
                else_block,
            } => {
                expect(ast, functions, condition, locals, *len, Type::Bool)?;
                let before_branches = *len;
                check_block(ast, functions, &then_block, return_type, locals, len)?;
                *len = before_branches;
                check_block(ast, functions, &else_block, return_type, locals, len)?;
                *len = before_branches;
            }
            Stmt::While { condition, body } => {
                expect(ast, functions, condition, locals, *len, Type::Bool)?;
                let before_body = *len;
                check_block(ast, functions, &body, return_type, locals, len)?;
                *len = before_body;
            }
        }
    }
    *len = scope;
    Ok(())
}
fn expression<'s>(
    ast: &mut Ast<'s>,
    functions: &[Option<crate::Function<'s>>],
    id: ExprId,
    locals: &[Option<Local<'s>>; 128],
    len: usize,
) -> Result<Type, CompileError> {
    let span = ast.expr_span(id);
    let ty = match ast.expr(id) {
        Expr::Integer(_) => Type::I64,
        Expr::Boolean(_) => Type::Bool,
        Expr::Name(name) => find(locals, len, name)
            .map(|v| v.ty)
            .ok_or_else(|| CompileError::new(ErrorCode::UnknownName, span, "невідоме ім'я"))?,
        Expr::Unary { op, value } => {
            let needed = if op == UnaryOp::Neg {
                Type::I64
            } else {
                Type::Bool
            };
            expect(ast, functions, value, locals, len, needed)?;
            needed
        }
        Expr::Binary { op, left, right } => {
            let operand = match op {
                BinaryOp::Add
                | BinaryOp::Sub
                | BinaryOp::Mul
                | BinaryOp::Div
                | BinaryOp::Rem
                | BinaryOp::Lt
                | BinaryOp::Le
                | BinaryOp::Gt
                | BinaryOp::Ge => Type::I64,
                BinaryOp::And | BinaryOp::Or => Type::Bool,
                BinaryOp::Eq | BinaryOp::Ne => expression(ast, functions, left, locals, len)?,
            };
            expect(ast, functions, left, locals, len, operand)?;
            expect(ast, functions, right, locals, len, operand)?;
            match op {
                BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul | BinaryOp::Div | BinaryOp::Rem => {
                    Type::I64
                }
                _ => Type::Bool,
            }
        }
        Expr::Call {
            name,
            args,
            len: count,
        } => {
            if name == "nova_print_byte" {
                if count != 1 {
                    return Err(CompileError::new(
                        ErrorCode::TypeMismatch,
                        span,
                        "nova_print_byte очікує один аргумент",
                    ));
                }
                expect(ast, functions, args[0], locals, len, Type::I64)?;
                ast.set_type(id, Type::I64);
                return Ok(Type::I64);
            }
            if name == "nova_yield" {
                if count != 0 {
                    return Err(CompileError::new(
                        ErrorCode::TypeMismatch,
                        span,
                        "nova_yield не приймає аргументів",
                    ));
                }
                ast.set_type(id, Type::I64);
                return Ok(Type::I64);
            }
            let function = functions
                .iter()
                .flatten()
                .find(|f| f.name == name)
                .ok_or_else(|| {
                    CompileError::new(ErrorCode::UnknownName, span, "невідома функція")
                })?;
            if count != function.param_count {
                return Err(CompileError::new(
                    ErrorCode::TypeMismatch,
                    span,
                    "неправильна кількість аргументів",
                ));
            }
            for i in 0..count as usize {
                expect(ast, functions, args[i], locals, len, function.params[i].ty)?;
            }
            function.return_type
        }
    };
    ast.set_type(id, ty);
    Ok(ty)
}
fn expect<'s>(
    ast: &mut Ast<'s>,
    functions: &[Option<crate::Function<'s>>],
    id: ExprId,
    locals: &[Option<Local<'s>>; 128],
    len: usize,
    wanted: Type,
) -> Result<(), CompileError> {
    let got = expression(ast, functions, id, locals, len)?;
    if got != wanted {
        Err(CompileError::new(
            ErrorCode::TypeMismatch,
            ast.expr_span(id),
            "невідповідний тип виразу",
        ))
    } else {
        Ok(())
    }
}
fn find<'a, 's>(locals: &'a [Option<Local<'s>>; 128], len: usize, name: &str) -> Option<Local<'s>> {
    locals[..len]
        .iter()
        .rev()
        .flatten()
        .find(|v| v.name == name)
        .copied()
}
fn insert<'s>(
    locals: &mut [Option<Local<'s>>; 128],
    len: &mut usize,
    local: Local<'s>,
    span: Span,
) -> Result<(), CompileError> {
    if locals[..*len]
        .iter()
        .flatten()
        .any(|v| v.name == local.name)
    {
        return Err(CompileError::new(
            ErrorCode::DuplicateName,
            span,
            "ім'я вже оголошене",
        ));
    }
    if *len == locals.len() {
        return Err(CompileError::new(
            ErrorCode::Capacity,
            span,
            "забагато локальних змінних",
        ));
    }
    locals[*len] = Some(local);
    *len += 1;
    Ok(())
}
