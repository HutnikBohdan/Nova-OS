use crate::{
    ast::{Ast, BinaryOp, Block, Expr, ExprId, Stmt, UnaryOp},
    CompileError, ErrorCode, IrProgram, Span,
};

pub const MAX_CODE: usize = 8192;
const MAX_SYMBOLS: usize = 32;
const MAX_RELOCS: usize = 128;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RelocationKind {
    PcRelative32,
}
#[derive(Clone, Copy, Debug)]
pub struct Relocation<'s> {
    pub offset: u32,
    pub target: &'s str,
    pub kind: RelocationKind,
}
#[derive(Clone, Copy, Debug)]
pub struct Symbol<'s> {
    pub name: &'s str,
    pub offset: u32,
}
pub struct CodeImage<'a, 's> {
    pub bytes: &'a [u8],
    pub entry_offset: u32,
    pub symbols: [Option<Symbol<'s>>; MAX_SYMBOLS],
    pub symbol_count: u8,
}

#[derive(Clone, Copy)]
struct Local<'s> {
    name: &'s str,
    offset: i32,
}
pub struct Emitter<'a, 's> {
    ast: &'a Ast<'s>,
    _ir: &'a IrProgram,
    output: &'a mut [u8],
    at: usize,
    symbols: [Option<Symbol<'s>>; MAX_SYMBOLS],
    symbol_len: usize,
    relocs: [Option<Relocation<'s>>; MAX_RELOCS],
    reloc_len: usize,
}
impl<'a, 's> Emitter<'a, 's> {
    pub fn new(ast: &'a Ast<'s>, ir: &'a IrProgram, output: &'a mut [u8]) -> Self {
        Self {
            ast,
            _ir: ir,
            output,
            at: 0,
            symbols: [None; MAX_SYMBOLS],
            symbol_len: 0,
            relocs: [None; MAX_RELOCS],
            reloc_len: 0,
        }
    }
    pub fn emit(mut self) -> Result<CodeImage<'a, 's>, CompileError> {
        let main = *self.ast.function("main").ok_or_else(|| {
            CompileError::new(
                ErrorCode::MissingMain,
                Span::default(),
                "потрібна функція main",
            )
        })?;
        self.function(&main)?;
        for function in self.ast.functions() {
            if function.name != "main" {
                self.function(function)?;
            }
        }
        self.resolve()?;
        let entry_offset = self.symbols[..self.symbol_len]
            .iter()
            .flatten()
            .find(|s| s.name == "main")
            .ok_or_else(|| {
                CompileError::new(
                    ErrorCode::MissingMain,
                    Span::default(),
                    "потрібна функція main",
                )
            })?
            .offset;
        Ok(CodeImage {
            bytes: &self.output[..self.at],
            entry_offset,
            symbols: self.symbols,
            symbol_count: self.symbol_len as u8,
        })
    }
    fn function(&mut self, function: &crate::Function<'s>) -> Result<(), CompileError> {
        self.add_symbol(function.name)?;
        let mut locals: [Option<Local<'s>>; 128] = [None; 128];
        let mut local_len = 0;
        for parameter in &function.params[..function.param_count as usize] {
            add_local(&mut locals, &mut local_len, parameter.name)?;
        }
        collect_locals(self.ast, &function.body, &mut locals, &mut local_len)?;
        self.bytes(&[0x55, 0x48, 0x89, 0xe5])?;
        let stack = ((local_len * 8 + 15) & !15) as u32;
        if stack != 0 {
            self.bytes(&[0x48, 0x81, 0xec])?;
            self.u32(stack)?;
        }
        for (index, local) in locals[..function.param_count as usize]
            .iter()
            .flatten()
            .enumerate()
        {
            self.store_argument(index, local.offset)?;
        }
        self.block(&function.body, &locals, local_len)?;
        self.bytes(&[0x31, 0xc0])?;
        self.epilogue()
    }
    fn block(
        &mut self,
        block: &Block,
        locals: &[Option<Local<'s>>; 128],
        local_len: usize,
    ) -> Result<(), CompileError> {
        for id in block.ids() {
            match self.ast.stmt(*id) {
                Stmt::Let { name, value, .. } | Stmt::Assign { name, value } => {
                    self.expr(value, locals, local_len)?;
                    let offset = find_local(locals, local_len, name).unwrap().offset;
                    self.store_rax(offset)?;
                }
                Stmt::Expr(value) => self.expr(value, locals, local_len)?,
                Stmt::Return(value) => {
                    self.expr(value, locals, local_len)?;
                    self.epilogue()?;
                }
                Stmt::If {
                    condition,
                    then_block,
                    else_block,
                } => {
                    self.expr(condition, locals, local_len)?;
                    self.bytes(&[0x48, 0x85, 0xc0, 0x0f, 0x84])?;
                    let false_jump = self.reserve_i32()?;
                    self.block(&then_block, locals, local_len)?;
                    self.byte(0xe9)?;
                    let end_jump = self.reserve_i32()?;
                    self.patch_relative(false_jump, self.at)?;
                    self.block(&else_block, locals, local_len)?;
                    self.patch_relative(end_jump, self.at)?;
                }
                Stmt::While { condition, body } => {
                    let start = self.at;
                    self.expr(condition, locals, local_len)?;
                    self.bytes(&[0x48, 0x85, 0xc0, 0x0f, 0x84])?;
                    let end = self.reserve_i32()?;
                    self.block(&body, locals, local_len)?;
                    self.byte(0xe9)?;
                    let back = self.reserve_i32()?;
                    self.patch_relative(back, start)?;
                    self.patch_relative(end, self.at)?;
                }
            }
        }
        Ok(())
    }
    fn expr(
        &mut self,
        id: ExprId,
        locals: &[Option<Local<'s>>; 128],
        local_len: usize,
    ) -> Result<(), CompileError> {
        match self.ast.expr(id) {
            Expr::Integer(value) => {
                self.bytes(&[0x48, 0xb8])?;
                self.u64(value as u64)?;
            }
            Expr::Boolean(value) => {
                self.byte(0xb8)?;
                self.u32(value as u32)?;
            }
            Expr::Name(name) => self.load_rax(
                find_local(locals, local_len, name)
                    .ok_or_else(|| {
                        CompileError::new(
                            ErrorCode::Internal,
                            self.ast.expr_span(id),
                            "локальну змінну втрачено",
                        )
                    })?
                    .offset,
            )?,
            Expr::Unary { op, value } => {
                self.expr(value, locals, local_len)?;
                self.bytes(if op == UnaryOp::Neg {
                    &[0x48, 0xf7, 0xd8]
                } else {
                    &[0x48, 0x83, 0xf0, 0x01]
                })?;
            }
            Expr::Binary { op, left, right } => {
                self.expr(left, locals, local_len)?;
                self.byte(0x50)?;
                self.expr(right, locals, local_len)?;
                self.byte(0x59)?;
                match op {
                    BinaryOp::Add => self.bytes(&[0x48, 0x01, 0xc8])?,
                    BinaryOp::Sub => self.bytes(&[0x48, 0x29, 0xc1, 0x48, 0x89, 0xc8])?,
                    BinaryOp::Mul => self.bytes(&[0x48, 0x0f, 0xaf, 0xc1])?,
                    BinaryOp::Div | BinaryOp::Rem => {
                        self.bytes(&[0x48, 0x91, 0x48, 0x99, 0x48, 0xf7, 0xf9])?;
                        if op == BinaryOp::Rem {
                            self.bytes(&[0x48, 0x89, 0xd0])?;
                        }
                    }
                    BinaryOp::And => self.bytes(&[0x48, 0x21, 0xc8])?,
                    BinaryOp::Or => self.bytes(&[0x48, 0x09, 0xc8])?,
                    compare => {
                        self.bytes(&[0x48, 0x39, 0xc1, 0x0f])?;
                        self.byte(match compare {
                            BinaryOp::Eq => 0x94,
                            BinaryOp::Ne => 0x95,
                            BinaryOp::Lt => 0x9c,
                            BinaryOp::Le => 0x9e,
                            BinaryOp::Gt => 0x9f,
                            BinaryOp::Ge => 0x9d,
                            _ => unreachable!(),
                        })?;
                        self.bytes(&[0xc0, 0x48, 0x0f, 0xb6, 0xc0])?;
                    }
                }
            }
            Expr::Call { name, args, len } => {
                if name == "nova_print_byte" {
                    self.expr(args[0], locals, local_len)?;
                    self.byte(0xa2)?;
                    self.u64(0x0060_0000)?;
                    self.bytes(&[0x48, 0xb8])?;
                    self.u64(4)?;
                    self.bytes(&[0x48, 0xbf])?;
                    self.u64(0x0060_0000)?;
                    self.bytes(&[0x48, 0xbe])?;
                    self.u64(1)?;
                    self.bytes(&[0xcd, 0x80])?;
                    return Ok(());
                }
                if name == "nova_yield" {
                    self.bytes(&[0x48, 0xb8])?;
                    self.u64(2)?;
                    self.bytes(&[0xcd, 0x80, 0x31, 0xc0])?;
                    return Ok(());
                }
                for argument in &args[..len as usize] {
                    self.expr(*argument, locals, local_len)?;
                    self.byte(0x50)?;
                }
                for index in (0..len as usize).rev() {
                    self.byte(match index {
                        0 => 0x5f,
                        1 => 0x5e,
                        2 => 0x5a,
                        3 => 0x59,
                        _ => {
                            return Err(CompileError::new(
                                ErrorCode::Unsupported,
                                self.ast.expr_span(id),
                                "у машинному коді підтримано до 4 аргументів",
                            ))
                        }
                    })?;
                }
                self.byte(0xe8)?;
                let offset = self.reserve_i32()?;
                self.add_relocation(offset, name)?;
            }
        }
        Ok(())
    }
    fn add_symbol(&mut self, name: &'s str) -> Result<(), CompileError> {
        if self.symbol_len == MAX_SYMBOLS {
            return Err(capacity());
        }
        self.symbols[self.symbol_len] = Some(Symbol {
            name,
            offset: self.at as u32,
        });
        self.symbol_len += 1;
        Ok(())
    }
    fn add_relocation(&mut self, offset: usize, target: &'s str) -> Result<(), CompileError> {
        if self.reloc_len == MAX_RELOCS {
            return Err(capacity());
        }
        self.relocs[self.reloc_len] = Some(Relocation {
            offset: offset as u32,
            target,
            kind: RelocationKind::PcRelative32,
        });
        self.reloc_len += 1;
        Ok(())
    }
    fn resolve(&mut self) -> Result<(), CompileError> {
        for relocation in self.relocs[..self.reloc_len].iter().flatten() {
            let target = self.symbols[..self.symbol_len]
                .iter()
                .flatten()
                .find(|s| s.name == relocation.target)
                .ok_or_else(|| {
                    CompileError::new(
                        ErrorCode::UnknownName,
                        Span::default(),
                        "невідома функція під час компонування",
                    )
                })?;
            let value = target.offset as i64 - (relocation.offset as i64 + 4);
            self.output[relocation.offset as usize..relocation.offset as usize + 4]
                .copy_from_slice(&(value as i32).to_le_bytes());
        }
        Ok(())
    }
    fn store_argument(&mut self, index: usize, offset: i32) -> Result<(), CompileError> {
        self.bytes(match index {
            0 => &[0x48, 0x89, 0xbd],
            1 => &[0x48, 0x89, 0xb5],
            2 => &[0x48, 0x89, 0x95],
            3 => &[0x48, 0x89, 0x8d],
            _ => {
                return Err(CompileError::new(
                    ErrorCode::Unsupported,
                    Span::default(),
                    "підтримано до 4 параметрів",
                ))
            }
        })?;
        self.i32(-offset)
    }
    fn load_rax(&mut self, offset: i32) -> Result<(), CompileError> {
        self.bytes(&[0x48, 0x8b, 0x85])?;
        self.i32(-offset)
    }
    fn store_rax(&mut self, offset: i32) -> Result<(), CompileError> {
        self.bytes(&[0x48, 0x89, 0x85])?;
        self.i32(-offset)
    }
    fn epilogue(&mut self) -> Result<(), CompileError> {
        self.bytes(&[0xc9, 0xc3])
    }
    fn patch_relative(&mut self, at: usize, target: usize) -> Result<(), CompileError> {
        let value = target as i64 - (at as i64 + 4);
        self.output[at..at + 4].copy_from_slice(&(value as i32).to_le_bytes());
        Ok(())
    }
    fn reserve_i32(&mut self) -> Result<usize, CompileError> {
        let at = self.at;
        self.u32(0)?;
        Ok(at)
    }
    fn byte(&mut self, value: u8) -> Result<(), CompileError> {
        if self.at == self.output.len() {
            return Err(CompileError::new(
                ErrorCode::OutputTooSmall,
                Span::default(),
                "замалий буфер машинного коду",
            ));
        }
        self.output[self.at] = value;
        self.at += 1;
        Ok(())
    }
    fn bytes(&mut self, values: &[u8]) -> Result<(), CompileError> {
        for value in values {
            self.byte(*value)?;
        }
        Ok(())
    }
    fn u32(&mut self, value: u32) -> Result<(), CompileError> {
        self.bytes(&value.to_le_bytes())
    }
    fn i32(&mut self, value: i32) -> Result<(), CompileError> {
        self.bytes(&value.to_le_bytes())
    }
    fn u64(&mut self, value: u64) -> Result<(), CompileError> {
        self.bytes(&value.to_le_bytes())
    }
}
fn collect_locals<'s>(
    ast: &Ast<'s>,
    block: &Block,
    locals: &mut [Option<Local<'s>>; 128],
    len: &mut usize,
) -> Result<(), CompileError> {
    for id in block.ids() {
        match ast.stmt(*id) {
            Stmt::Let { name, .. } => {
                if find_local(locals, *len, name).is_none() {
                    add_local(locals, len, name)?;
                }
            }
            Stmt::If {
                then_block,
                else_block,
                ..
            } => {
                collect_locals(ast, &then_block, locals, len)?;
                collect_locals(ast, &else_block, locals, len)?;
            }
            Stmt::While { body, .. } => collect_locals(ast, &body, locals, len)?,
            _ => {}
        }
    }
    Ok(())
}
fn add_local<'s>(
    locals: &mut [Option<Local<'s>>; 128],
    len: &mut usize,
    name: &'s str,
) -> Result<(), CompileError> {
    if *len == locals.len() {
        return Err(capacity());
    }
    locals[*len] = Some(Local {
        name,
        offset: ((*len + 1) * 8) as i32,
    });
    *len += 1;
    Ok(())
}
fn find_local<'s>(locals: &[Option<Local<'s>>; 128], len: usize, name: &str) -> Option<Local<'s>> {
    locals[..len]
        .iter()
        .flatten()
        .find(|v| v.name == name)
        .copied()
}
fn capacity() -> CompileError {
    CompileError::new(
        ErrorCode::Capacity,
        Span::default(),
        "перевищено місткість генератора коду",
    )
}
