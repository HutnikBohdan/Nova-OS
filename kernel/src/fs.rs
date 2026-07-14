use core::str;

const MAX_FILES: usize = 16;
const PATH_BYTES: usize = 48;
const DATA_BYTES: usize = 1024;

#[derive(Clone, Copy)]
struct File {
    used: bool,
    path: [u8; PATH_BYTES],
    path_len: usize,
    data: [u8; DATA_BYTES],
    len: usize,
}
const EMPTY: File = File {
    used: false,
    path: [0; PATH_BYTES],
    path_len: 0,
    data: [0; DATA_BYTES],
    len: 0,
};

pub struct RamFs {
    files: [File; MAX_FILES],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FsError {
    InvalidPath,
    NotFound,
    NoSpace,
    TooLarge,
}

impl RamFs {
    pub const fn new() -> Self {
        Self {
            files: [EMPTY; MAX_FILES],
        }
    }
    pub fn seed(&mut self) {
        let _ = self.write(
            "/ПРОЧИТАЙ-МЕНЕ.txt",
            "Nova OS працює. AI-оператор має системні можливості.\n".as_bytes(),
        );
        let _ = self.write(
            "/Проєкти/Nova.todo",
            "Побудувати компонувальник, постійне сховище та ізоляцію процесів.\n".as_bytes(),
        );
        let _ = self.write(
            "/Документи/Вітаємо.txt",
            "Ласкаво просимо до Nova OS. Натисніть F4, щоб відкрити Nova AI.\n".as_bytes(),
        );
        let _ = self.write("/Система/збірка.info", b"Nova OS 0.3 x86_64 Rust\n");
        let _ = self.write(
            "/Проєкти/Привіт.nv",
            b"fn main() -> i64 { nova_print_byte(65); nova_yield(); return 7; }",
        );
    }
    pub fn paths(&self, mut visit: impl FnMut(&str)) {
        for file in &self.files {
            if file.used {
                visit(str::from_utf8(&file.path[..file.path_len]).unwrap_or("?"));
            }
        }
    }
    pub fn export(&self, out: &mut [u8]) -> Result<usize, FsError> {
        if out.len() < 8 {
            return Err(FsError::TooLarge);
        }
        out.fill(0);
        out[..4].copy_from_slice(b"NVFS");
        let count = self.files.iter().filter(|file| file.used).count() as u16;
        out[4..6].copy_from_slice(&count.to_le_bytes());
        let mut at = 8usize;
        for file in self.files.iter().filter(|file| file.used) {
            let needed = 4 + file.path_len + file.len;
            if at + needed > out.len() {
                return Err(FsError::TooLarge);
            }
            out[at..at + 2].copy_from_slice(&(file.path_len as u16).to_le_bytes());
            out[at + 2..at + 4].copy_from_slice(&(file.len as u16).to_le_bytes());
            at += 4;
            out[at..at + file.path_len].copy_from_slice(&file.path[..file.path_len]);
            at += file.path_len;
            out[at..at + file.len].copy_from_slice(&file.data[..file.len]);
            at += file.len;
        }
        Ok(at)
    }
    pub fn import(&mut self, input: &[u8]) -> Result<(), FsError> {
        if input.len() < 8 || &input[..4] != b"NVFS" {
            return Err(FsError::InvalidPath);
        }
        let count = u16::from_le_bytes([input[4], input[5]]) as usize;
        if count > MAX_FILES {
            return Err(FsError::NoSpace);
        }
        self.files = [EMPTY; MAX_FILES];
        let mut at = 8usize;
        for _ in 0..count {
            if at + 4 > input.len() {
                return Err(FsError::TooLarge);
            }
            let path_len = u16::from_le_bytes([input[at], input[at + 1]]) as usize;
            let data_len = u16::from_le_bytes([input[at + 2], input[at + 3]]) as usize;
            at += 4;
            if path_len == 0
                || path_len > PATH_BYTES
                || data_len > DATA_BYTES
                || at + path_len + data_len > input.len()
            {
                return Err(FsError::TooLarge);
            }
            let path =
                str::from_utf8(&input[at..at + path_len]).map_err(|_| FsError::InvalidPath)?;
            at += path_len;
            let data = &input[at..at + data_len];
            at += data_len;
            self.write(path, data)?;
        }
        Ok(())
    }
    pub fn read(&self, path: &str) -> Result<&str, FsError> {
        let file = self.find(path).ok_or(FsError::NotFound)?;
        str::from_utf8(&file.data[..file.len]).map_err(|_| FsError::NotFound)
    }
    pub fn read_bytes(&self, path: &str) -> Result<&[u8], FsError> {
        let file = self.find(path).ok_or(FsError::NotFound)?;
        Ok(&file.data[..file.len])
    }
    pub fn write(&mut self, path: &str, data: &[u8]) -> Result<(), FsError> {
        validate(path, data)?;
        let slot = self
            .find_index(path)
            .or_else(|| self.files.iter().position(|f| !f.used))
            .ok_or(FsError::NoSpace)?;
        let file = &mut self.files[slot];
        *file = EMPTY;
        file.used = true;
        file.path_len = path.len();
        file.path[..path.len()].copy_from_slice(path.as_bytes());
        file.len = data.len();
        file.data[..data.len()].copy_from_slice(data);
        Ok(())
    }
    pub fn append(&mut self, path: &str, data: &[u8]) -> Result<(), FsError> {
        if self.find_index(path).is_none() {
            return self.write(path, data);
        }
        let file = &mut self.files[self.find_index(path).unwrap()];
        if file.len + data.len() > DATA_BYTES {
            return Err(FsError::TooLarge);
        }
        file.data[file.len..file.len + data.len()].copy_from_slice(data);
        file.len += data.len();
        Ok(())
    }
    pub fn delete(&mut self, path: &str) -> Result<(), FsError> {
        let i = self.find_index(path).ok_or(FsError::NotFound)?;
        self.files[i] = EMPTY;
        Ok(())
    }
    fn find(&self, path: &str) -> Option<&File> {
        self.find_index(path).map(|i| &self.files[i])
    }
    fn find_index(&self, path: &str) -> Option<usize> {
        self.files
            .iter()
            .position(|f| f.used && &f.path[..f.path_len] == path.as_bytes())
    }
}

fn validate(path: &str, data: &[u8]) -> Result<(), FsError> {
    if !path.starts_with('/') || path.len() < 2 || path.len() > PATH_BYTES {
        return Err(FsError::InvalidPath);
    }
    if data.len() > DATA_BYTES {
        return Err(FsError::TooLarge);
    }
    Ok(())
}
