#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Span {
    pub start: u32,
    pub end: u32,
}

impl Span {
    pub const fn new(start: usize, end: usize) -> Self {
        Self {
            start: start as u32,
            end: end as u32,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorCode {
    UnexpectedCharacter,
    UnexpectedToken,
    Capacity,
    UnknownName,
    DuplicateName,
    TypeMismatch,
    MissingMain,
    InvalidMain,
    Unsupported,
    OutputTooSmall,
    Internal,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Diagnostic {
    pub code: ErrorCode,
    pub span: Span,
    pub message: &'static str,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CompileError(pub(crate) Diagnostic);

impl CompileError {
    pub const fn new(code: ErrorCode, span: Span, message: &'static str) -> Self {
        Self(Diagnostic {
            code,
            span,
            message,
        })
    }
    pub const fn diagnostic(self) -> Diagnostic {
        self.0
    }
}
