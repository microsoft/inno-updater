/*-----------------------------------------------------------------------------------------
 *  Copyright (c) Microsoft Corporation. All rights reserved.
 *  Licensed under the MIT License. See LICENSE in the project root for license information.
 *----------------------------------------------------------------------------------------*/

use std::io;
use std::io::prelude::*;
use std::string;

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

pub fn write_utf8_string(
	writer: &mut Write,
	string: &String,
	capacity: usize,
) -> Result<(), io::Error> {
	let bytes = string.as_bytes();
	writer.write_all(&bytes)?;

	let rest = vec![0; capacity - bytes.len()];
	writer.write_all(&rest)?;

	Ok(())
}

pub fn to_utf16(value: &str) -> Vec<u16> {
	use std::ffi::OsStr;
	use std::iter::once;
	use std::os::windows::ffi::OsStrExt;

	OsStr::new(value).encode_wide().chain(once(0u16)).collect()
}

pub fn from_utf16(value: &[u16]) -> Result<String, io::Error> {
	use std::ffi::OsString;
	use std::os::windows::ffi::OsStringExt;

	let pos = value.iter().position(|&x| x == 0).unwrap_or(value.len());
	let value = &value[0..pos];

	OsString::from_wide(value)
		.into_string()
		.map_err(|_| io::Error::new(io::ErrorKind::Other, "Could convert from utf16"))
}
