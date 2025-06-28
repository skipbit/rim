use std::fs;
use std::io;

pub trait FileIO {
    fn read_file(&self, path: &str) -> io::Result<String>;
    fn write_file(&self, path: &str, content: &str) -> io::Result<()>;
}

pub struct LocalFileIO;

impl FileIO for LocalFileIO {
    fn read_file(&self, path: &str) -> io::Result<String> {
        fs::read_to_string(path)
    }

    fn write_file(&self, path: &str, content: &str) -> io::Result<()> {
        fs::write(path, content)
    }
}
