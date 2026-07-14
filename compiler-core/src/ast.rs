use crate::{CompileError, ErrorCode, Span, Type};

pub const MAX_FUNCTIONS: usize = 32;
pub const MAX_PARAMS: usize = 8;
pub const MAX_EXPRS: usize = 256;
pub const MAX_STMTS: usize = 128;
pub const MAX_BLOCK_STMTS: usize = 32;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ExprId(pub u16);
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StmtId(pub u16);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnaryOp {
    Neg,
    Not,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BinaryOp {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    And,
    Or,
}

#[derive(Clone, Copy, Debug)]
pub enum Expr<'s> {
    Integer(i64),
    Boolean(bool),
    Name(&'s str),
    Unary {
        op: UnaryOp,
        value: ExprId,
    },
    Binary {
        op: BinaryOp,
        left: ExprId,
        right: ExprId,
    },
    Call {
        name: &'s str,
        args: [ExprId; MAX_PARAMS],
        len: u8,
    },
}

#[derive(Clone, Copy, Debug)]
pub struct Block {
    pub statements: [StmtId; MAX_BLOCK_STMTS],
    pub len: u8,
}
impl Block {
    pub const fn new() -> Self {
        Self {
            statements: [StmtId(0); MAX_BLOCK_STMTS],
            len: 0,
        }
    }
    pub fn push(&mut self, id: StmtId, span: Span) -> Result<(), CompileError> {
        if self.len as usize == MAX_BLOCK_STMTS {
            return Err(CompileError::new(
                ErrorCode::Capacity,
                span,
                "забагато команд у блоці",
            ));
        }
        self.statements[self.len as usize] = id;
        self.len += 1;
        Ok(())
    }
    pub fn ids(&self) -> &[StmtId] {
        &self.statements[..self.len as usize]
    }
}
impl Default for Block {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy, Debug)]
pub enum Stmt<'s> {
    Let {
        name: &'s str,
        mutable: bool,
        ty: Type,
        value: ExprId,
    },
    Assign {
        name: &'s str,
        value: ExprId,
    },
    Expr(ExprId),
    Return(ExprId),
    If {
        condition: ExprId,
        then_block: Block,
        else_block: Block,
    },
    While {
        condition: ExprId,
        body: Block,
    },
}

#[derive(Clone, Copy, Debug)]
pub struct Param<'s> {
    pub name: &'s str,
    pub ty: Type,
    pub span: Span,
}
impl Default for Param<'_> {
    fn default() -> Self {
        Self {
            name: "",
            ty: Type::I64,
            span: Span::default(),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Function<'s> {
    pub name: &'s str,
    pub params: [Param<'s>; MAX_PARAMS],
    pub param_count: u8,
    pub return_type: Type,
    pub body: Block,
    pub span: Span,
}
impl Default for Function<'_> {
    fn default() -> Self {
        Self {
            name: "",
            params: [Param::default(); MAX_PARAMS],
            param_count: 0,
            return_type: Type::I64,
            body: Block::new(),
            span: Span::default(),
        }
    }
}

pub struct Ast<'s> {
    functions: [Function<'s>; MAX_FUNCTIONS],
    function_len: usize,
    exprs: [Option<Expr<'s>>; MAX_EXPRS],
    expr_spans: [Span; MAX_EXPRS],
    expr_types: [Type; MAX_EXPRS],
    expr_len: usize,
    stmts: [Option<Stmt<'s>>; MAX_STMTS],
    stmt_spans: [Span; MAX_STMTS],
    stmt_len: usize,
}
impl<'s> Ast<'s> {
    pub const fn new() -> Self {
        const F: Function<'static> = Function {
            name: "",
            params: [Param {
                name: "",
                ty: Type::I64,
                span: Span { start: 0, end: 0 },
            }; MAX_PARAMS],
            param_count: 0,
            return_type: Type::I64,
            body: Block::new(),
            span: Span { start: 0, end: 0 },
        };
        Self {
            functions: [F; MAX_FUNCTIONS],
            function_len: 0,
            exprs: [None; MAX_EXPRS],
            expr_spans: [Span { start: 0, end: 0 }; MAX_EXPRS],
            expr_types: [Type::Unknown; MAX_EXPRS],
            expr_len: 0,
            stmts: [None; MAX_STMTS],
            stmt_spans: [Span { start: 0, end: 0 }; MAX_STMTS],
            stmt_len: 0,
        }
    }
    pub fn clear(&mut self) {
        self.function_len = 0;
        self.expr_len = 0;
        self.stmt_len = 0;
    }
    pub fn push_expr(&mut self, expr: Expr<'s>, span: Span) -> Result<ExprId, CompileError> {
        if self.expr_len == MAX_EXPRS {
            return Err(CompileError::new(
                ErrorCode::Capacity,
                span,
                "забагато виразів",
            ));
        }
        let id = ExprId(self.expr_len as u16);
        self.exprs[self.expr_len] = Some(expr);
        self.expr_spans[self.expr_len] = span;
        self.expr_len += 1;
        Ok(id)
    }
    pub fn push_stmt(&mut self, stmt: Stmt<'s>, span: Span) -> Result<StmtId, CompileError> {
        if self.stmt_len == MAX_STMTS {
            return Err(CompileError::new(
                ErrorCode::Capacity,
                span,
                "забагато команд",
            ));
        }
        let id = StmtId(self.stmt_len as u16);
        self.stmts[self.stmt_len] = Some(stmt);
        self.stmt_spans[self.stmt_len] = span;
        self.stmt_len += 1;
        Ok(id)
    }
    pub fn push_function(&mut self, function: Function<'s>) -> Result<(), CompileError> {
        if self.function_len == MAX_FUNCTIONS {
            return Err(CompileError::new(
                ErrorCode::Capacity,
                function.span,
                "забагато функцій",
            ));
        }
        if self.functions().any(|f| f.name == function.name) {
            return Err(CompileError::new(
                ErrorCode::DuplicateName,
                function.span,
                "функція вже визначена",
            ));
        }
        self.functions[self.function_len] = function;
        self.function_len += 1;
        Ok(())
    }
    pub fn functions(&self) -> impl Iterator<Item = &Function<'s>> {
        self.functions[..self.function_len].iter()
    }
    pub(crate) fn function(&self, name: &str) -> Option<&Function<'s>> {
        self.functions().find(|f| f.name == name)
    }
    pub(crate) fn expr(&self, id: ExprId) -> Expr<'s> {
        self.exprs[id.0 as usize].unwrap()
    }
    pub(crate) fn stmt(&self, id: StmtId) -> Stmt<'s> {
        self.stmts[id.0 as usize].unwrap()
    }
    pub(crate) fn expr_span(&self, id: ExprId) -> Span {
        self.expr_spans[id.0 as usize]
    }
    pub(crate) fn stmt_span(&self, id: StmtId) -> Span {
        self.stmt_spans[id.0 as usize]
    }
    pub(crate) fn set_type(&mut self, id: ExprId, ty: Type) {
        self.expr_types[id.0 as usize] = ty;
    }
}
impl Default for Ast<'_> {
    fn default() -> Self {
        Self::new()
    }
}
