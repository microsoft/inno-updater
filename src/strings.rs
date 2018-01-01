use std::io;
use std::string;
use std::io::prelude::*;

#[derive(Debug)]
pub enum ReadUtf8StringError {
	IOError(io::Error),
	UTF8Error(string::FromUtf8Error),
}

pub fn read_utf8_string(reader: &mut Read, capacity: usize) -> Result<String, ReadUtf8StringError> {
	let mut vec = vec![0; capacity];

	reader
		.read_exact(&mut vec)
		.map_err(|err| ReadUtf8StringError::IOError(err))
		.and_then(|_| {
			let pos = vec.iter().position(|&x| x == 0).unwrap_or(64);
			let bar = &vec[0..pos];
			String::from_utf8(Vec::from(bar)).map_err(|err| ReadUtf8StringError::UTF8Error(err))
		})
}
