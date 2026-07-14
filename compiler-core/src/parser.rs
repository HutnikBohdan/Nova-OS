use crate::{
    ast::{Ast, BinaryOp, Block, Expr, ExprId, Function, Param, Stmt, UnaryOp, MAX_PARAMS},
    CompileError, ErrorCode, Lexer, Span, Token, TokenKind, Type,
};

pub struct Parser<'a, 's> {
    lexer: Lexer<'s>,
    current: Token<'s>,
    ast: &'a mut Ast<'s>,
}
impl<'a, 's> Parser<'a, 's> {
    pub fn new(source: &'s str, ast: &'a mut Ast<'s>) -> Self {
        Self {
            lexer: Lexer::new(source),
            current: Token {
                kind: TokenKind::Eof,
                text: "",
                span: Span::default(),
            },
            ast,
        }
    }
    pub fn parse_program(mut self) -> Result<(), CompileError> {
        self.advance()?;
        while self.current.kind != TokenKind::Eof {
            self.parse_function()?;
        }
        if self.ast.function("main").is_none() {
            return Err(self.error(ErrorCode::MissingMain, "потрібна функція main"));
        }
        Ok(())
    }
    fn parse_function(&mut self) -> Result<(), CompileError> {
        let start = self.expect(TokenKind::Fn)?.span.start;
        let name = self.expect(TokenKind::Ident)?;
        self.expect(TokenKind::LParen)?;
        let mut params = [Param::default(); MAX_PARAMS];
        let mut count = 0usize;
        if self.current.kind != TokenKind::RParen {
            loop {
                if count == MAX_PARAMS {
                    return Err(self.error(ErrorCode::Capacity, "забагато параметрів"));
                }
                let parameter = self.expect(TokenKind::Ident)?;
                self.expect(TokenKind::Colon)?;
                params[count] = Param {
                    name: parameter.text,
                    ty: self.parse_type()?,
                    span: parameter.span,
                };
                count += 1;
                if !self.consume(TokenKind::Comma)? {
                    break;
                }
            }
        }
        self.expect(TokenKind::RParen)?;
        self.expect(TokenKind::Arrow)?;
        let return_type = self.parse_type()?;
        let body = self.parse_block()?;
        self.ast.push_function(Function {
            name: name.text,
            params,
            param_count: count as u8,
            return_type,
            body,
            span: Span {
                start,
                end: self.current.span.start,
            },
        })
    }
    fn parse_type(&mut self) -> Result<Type, CompileError> {
        let ty = match self.current.kind {
            TokenKind::I64 => Type::I64,
            TokenKind::Bool => Type::Bool,
            _ => return Err(self.error(ErrorCode::UnexpectedToken, "очікувався тип i64 або bool")),
        };
        self.advance()?;
        Ok(ty)
    }
    fn parse_block(&mut self) -> Result<Block, CompileError> {
        self.expect(TokenKind::LBrace)?;
        let mut block = Block::new();
        while self.current.kind != TokenKind::RBrace {
            if self.current.kind == TokenKind::Eof {
                return Err(self.error(ErrorCode::UnexpectedToken, "незакритий блок"));
            }
            let (id, span) = self.parse_stmt()?;
            block.push(id, span)?;
        }
        self.expect(TokenKind::RBrace)?;
        Ok(block)
    }
    fn parse_stmt(&mut self) -> Result<(crate::StmtId, Span), CompileError> {
        let start = self.current.span.start;
        let stmt = match self.current.kind {
            TokenKind::Let => {
                self.advance()?;
                let mutable = self.consume(TokenKind::Mut)?;
                let name = self.expect(TokenKind::Ident)?;
                self.expect(TokenKind::Colon)?;
                let ty = self.parse_type()?;
                self.expect(TokenKind::Equal)?;
                let value = self.parse_expr()?;
                self.expect(TokenKind::Semicolon)?;
                Stmt::Let {
                    name: name.text,
                    mutable,
                    ty,
                    value,
                }
            }
            TokenKind::Return => {
                self.advance()?;
                let value = self.parse_expr()?;
                self.expect(TokenKind::Semicolon)?;
                Stmt::Return(value)
            }
            TokenKind::If => {
                self.advance()?;
                let condition = self.parse_expr()?;
                let then_block = self.parse_block()?;
                let else_block = if self.consume(TokenKind::Else)? {
                    self.parse_block()?
                } else {
                    Block::new()
                };
                Stmt::If {
                    condition,
                    then_block,
                    else_block,
                }
            }
            TokenKind::While => {
                self.advance()?;
                let condition = self.parse_expr()?;
                let body = self.parse_block()?;
                Stmt::While { condition, body }
            }
            TokenKind::Ident => {
                let saved = self.current;
                self.advance()?;
                if self.current.kind == TokenKind::Equal {
                    self.advance()?;
                    let value = self.parse_expr()?;
                    self.expect(TokenKind::Semicolon)?;
                    Stmt::Assign {
                        name: saved.text,
                        value,
                    }
                } else {
                    let expression = self.parse_expr_after_name(saved)?;
                    self.expect(TokenKind::Semicolon)?;
                    Stmt::Expr(expression)
                }
            }
            _ => {
                let value = self.parse_expr()?;
                self.expect(TokenKind::Semicolon)?;
                Stmt::Expr(value)
            }
        };
        let span = Span {
            start,
            end: self.current.span.start,
        };
        let id = self.ast.push_stmt(stmt, span)?;
        Ok((id, span))
    }
    fn parse_expr(&mut self) -> Result<ExprId, CompileError> {
        self.parse_binary(0)
    }
    fn parse_binary(&mut self, min_precedence: u8) -> Result<ExprId, CompileError> {
        let mut left = self.parse_unary()?;
        loop {
            let Some((op, precedence)) = binary(self.current.kind) else {
                break;
            };
            if precedence < min_precedence {
                break;
            }
            let start = self.ast.expr_span(left).start;
            self.advance()?;
            let right = self.parse_binary(precedence + 1)?;
            left = self.ast.push_expr(
                Expr::Binary { op, left, right },
                Span {
                    start,
                    end: self.ast.expr_span(right).end,
                },
            )?;
        }
        Ok(left)
    }
    fn parse_unary(&mut self) -> Result<ExprId, CompileError> {
        let op = match self.current.kind {
            TokenKind::Minus => Some(UnaryOp::Neg),
            TokenKind::Bang => Some(UnaryOp::Not),
            _ => None,
        };
        if let Some(op) = op {
            let start = self.current.span.start;
            self.advance()?;
            let value = self.parse_unary()?;
            return self.ast.push_expr(
                Expr::Unary { op, value },
                Span {
                    start,
                    end: self.ast.expr_span(value).end,
                },
            );
        }
        self.parse_primary()
    }
    fn parse_primary(&mut self) -> Result<ExprId, CompileError> {
        let token = self.current;
        self.advance()?;
        match token.kind {
            TokenKind::Integer => {
                let value = parse_i64(token.text).ok_or_else(|| {
                    CompileError::new(
                        ErrorCode::Unsupported,
                        token.span,
                        "число поза діапазоном i64",
                    )
                })?;
                self.ast.push_expr(Expr::Integer(value), token.span)
            }
            TokenKind::True | TokenKind::False => self
                .ast
                .push_expr(Expr::Boolean(token.kind == TokenKind::True), token.span),
            TokenKind::Ident => self.parse_expr_after_name(token),
            TokenKind::LParen => {
                let value = self.parse_expr()?;
                self.expect(TokenKind::RParen)?;
                Ok(value)
            }
            _ => Err(CompileError::new(
                ErrorCode::UnexpectedToken,
                token.span,
                "очікувався вираз",
            )),
        }
    }
    fn parse_expr_after_name(&mut self, name: Token<'s>) -> Result<ExprId, CompileError> {
        if !self.consume(TokenKind::LParen)? {
            return self.ast.push_expr(Expr::Name(name.text), name.span);
        }
        let mut args = [ExprId(0); MAX_PARAMS];
        let mut len = 0usize;
        if self.current.kind != TokenKind::RParen {
            loop {
                if len == MAX_PARAMS {
                    return Err(self.error(ErrorCode::Capacity, "забагато аргументів"));
                }
                args[len] = self.parse_expr()?;
                len += 1;
                if !self.consume(TokenKind::Comma)? {
                    break;
                }
            }
        }
        let end = self.expect(TokenKind::RParen)?.span.end;
        self.ast.push_expr(
            Expr::Call {
                name: name.text,
                args,
                len: len as u8,
            },
            Span {
                start: name.span.start,
                end,
            },
        )
    }
    fn advance(&mut self) -> Result<(), CompileError> {
        self.current = self.lexer.next()?;
        Ok(())
    }
    fn expect(&mut self, kind: TokenKind) -> Result<Token<'s>, CompileError> {
        if self.current.kind != kind {
            return Err(self.error(ErrorCode::UnexpectedToken, "неочікуваний елемент програми"));
        }
        let token = self.current;
        self.advance()?;
        Ok(token)
    }
    fn consume(&mut self, kind: TokenKind) -> Result<bool, CompileError> {
        if self.current.kind == kind {
            self.advance()?;
            Ok(true)
        } else {
            Ok(false)
        }
    }
    fn error(&self, code: ErrorCode, message: &'static str) -> CompileError {
        CompileError::new(code, self.current.span, message)
    }
}

fn binary(kind: TokenKind) -> Option<(BinaryOp, u8)> {
    Some(match kind {
        TokenKind::OrOr => (BinaryOp::Or, 1),
        TokenKind::AndAnd => (BinaryOp::And, 2),
        TokenKind::EqualEqual => (BinaryOp::Eq, 3),
        TokenKind::BangEqual => (BinaryOp::Ne, 3),
        TokenKind::Less => (BinaryOp::Lt, 4),
        TokenKind::LessEqual => (BinaryOp::Le, 4),
        TokenKind::Greater => (BinaryOp::Gt, 4),
        TokenKind::GreaterEqual => (BinaryOp::Ge, 4),
        TokenKind::Plus => (BinaryOp::Add, 5),
        TokenKind::Minus => (BinaryOp::Sub, 5),
        TokenKind::Star => (BinaryOp::Mul, 6),
        TokenKind::Slash => (BinaryOp::Div, 6),
        TokenKind::Percent => (BinaryOp::Rem, 6),
        _ => return None,
    })
}
fn parse_i64(text: &str) -> Option<i64> {
    let mut value = 0i64;
    for byte in text.bytes() {
        value = value.checked_mul(10)?.checked_add((byte - b'0') as i64)?;
    }
    Some(value)
}
