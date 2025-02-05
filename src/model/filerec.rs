/*-----------------------------------------------------------------------------------------
 *  Copyright (c) Microsoft Corporation. All rights reserved.
 *  Licensed under the MIT License. See LICENSE in the project root for license information.
 *----------------------------------------------------------------------------------------*/

use byteorder::{ByteOrder, LittleEndian, ReadBytesExt, WriteBytesExt};
use std::io::prelude::*;
use std::path::Path;
use std::string::String;
use std::{error, fmt};

#[derive(Copy, Clone)]
pub enum UninstallRecTyp {
	UserDefined = 0x01,
	StartInstall = 0x10,
	EndInstall = 0x11,
	CompiledCode = 0x20,
	Run = 0x80,
	DeleteDirOrFiles = 0x81,
	DeleteFile = 0x82,
	DeleteGroupOrItem = 0x83,
	IniDeleteEntry = 0x84,
	IniDeleteSection = 0x85,
	RegDeleteEntireKey = 0x86,
	RegClearValue = 0x87,
	RegDeleteKeyIfEmpty = 0x88,
	RegDeleteValue = 0x89,
	DecrementSharedCount = 0x8A,
	RefreshFileAssoc = 0x8B,
	MutexCheck = 0x8C,
}

impl UninstallRecTyp {
	fn from(i: u16) -> UninstallRecTyp {
		match i {
			0x01 => UninstallRecTyp::UserDefined,
			0x10 => UninstallRecTyp::StartInstall,
			0x11 => UninstallRecTyp::EndInstall,
			0x20 => UninstallRecTyp::CompiledCode,
			0x80 => UninstallRecTyp::Run,
			0x81 => UninstallRecTyp::DeleteDirOrFiles,
			0x82 => UninstallRecTyp::DeleteFile,
			0x83 => UninstallRecTyp::DeleteGroupOrItem,
			0x84 => UninstallRecTyp::IniDeleteEntry,
			0x85 => UninstallRecTyp::IniDeleteSection,
			0x86 => UninstallRecTyp::RegDeleteEntireKey,
			0x87 => UninstallRecTyp::RegClearValue,
			0x88 => UninstallRecTyp::RegDeleteKeyIfEmpty,
			0x89 => UninstallRecTyp::RegDeleteValue,
			0x8A => UninstallRecTyp::DecrementSharedCount,
			0x8B => UninstallRecTyp::RefreshFileAssoc,
			0x8C => UninstallRecTyp::MutexCheck,
			_ => panic!(""),
		}
	}
}

#[derive(Clone)]
pub struct FileRec {
	pub typ: UninstallRecTyp,
	extra_data: u32,
	data: Vec<u8>,
}

impl fmt::Debug for FileRec {
	fn fmt(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
		write!(
			formatter,
			"FileRec 0x{:x} 0x{:x} {} bytes",
			self.typ as u32,
			{ self.extra_data },
			self.data.len(),
		)
	}
}

#[derive(Debug, Clone)]
pub struct StringDecodeError<'a>(&'a str);

impl<'a> fmt::Display for StringDecodeError<'a> {
	fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
		write!(f, "String decode error: {}", self.0)
	}
}

impl<'a> error::Error for StringDecodeError<'a> {
	fn description(&self) -> &str {
		"StringDecodeError"
	}

	fn cause(&self) -> Option<&dyn error::Error> {
		None
	}
}

fn decode_strings<'a>(data: &[u8]) -> Result<Vec<String>, StringDecodeError<'a>> {
	let mut result: Vec<String> = Vec::with_capacity(10);
	let mut slice = data;

	loop {
		let reader: &mut dyn Read = &mut slice;
		let byte_result = reader
			.read_u8()
			.map_err(|_| StringDecodeError("Failed to parse file rec string header"))?;

		match byte_result {
			0x00..=0xfc => panic!("What 0x{:x}", byte_result),
			0xfd => panic!("What 0x{:x}", byte_result),
			0xfe => {
				let size = reader
					.read_i32::<LittleEndian>()
					.map_err(|_| StringDecodeError("Failed to parse file rec string size"))?;

				let size = -size as usize;

				if size > 0 {
					assert_eq!(size % 2, 0);

					let mut u16data: Vec<u16> = vec![0; size / 2];
					LittleEndian::read_u16_into(&slice[5..5 + size], &mut u16data);

					let string = String::from_utf16(&u16data)
						.map_err(|_| StringDecodeError("Failed to parse file rec data string"))?;
					result.push(string);
				}

				slice = &slice[5 + size..];
			}
			0xff => {
				if slice.len() != 1 {
					return Err(StringDecodeError("Invalid file rec string header length"));
				}
				return Ok(result);
			}
		}
	}
}

#[derive(Debug, Clone)]
pub struct StringEncodeError<'a>(&'a str);

impl<'a> fmt::Display for StringEncodeError<'a> {
	fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
		write!(f, "String encode error: {}", self.0)
	}
}

impl<'a> error::Error for StringEncodeError<'a> {
	fn description(&self) -> &str {
		"StringEncodeError"
	}

	fn cause(&self) -> Option<&dyn error::Error> {
		None
	}
}

fn encode_strings<'a>(strings: &[String]) -> Result<Vec<u8>, StringEncodeError<'a>> {
	let mut result: Vec<u8> = Vec::with_capacity(1024);

	for string in strings.iter() {
		let u16data: Vec<u16> = string.encode_utf16().collect();
		let size = u16data.len() * 2;

		result
			.write_u8(0xfe)
			.map_err(|_| StringEncodeError("Failed to write file rec string header"))?;

		result
			.write_i32::<LittleEndian>(-(size as i32))
			.map_err(|_| StringEncodeError("Failed to write file rec string size"))?;

		let start = result.len();
		let end = start + size;
		result.resize(end, 0);

		LittleEndian::write_u16_into(&u16data, &mut result[start..end]);
	}

	result
		.write_u8(0xff)
		.map_err(|_| StringEncodeError("Failed to write file rec string end"))?;

	Ok(result)
}

#[derive(Debug, Clone)]
pub struct FileRecParseError<'a>(&'a str);

impl<'a> fmt::Display for FileRecParseError<'a> {
	fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
		write!(f, "FileRec parse error: {}", self.0)
	}
}

impl<'a> error::Error for FileRecParseError<'a> {
	fn description(&self) -> &str {
		"FileRecParseError"
	}

	fn cause(&self) -> Option<&dyn error::Error> {
		None
	}
}

#[derive(Debug, Clone)]
pub struct FileRecWriteError<'a>(&'a str);

impl<'a> fmt::Display for FileRecWriteError<'a> {
	fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
		write!(f, "FileRec write error: {}", self.0)
	}
}

impl<'a> error::Error for FileRecWriteError<'a> {
	fn description(&self) -> &str {
		"FileRecWriteError"
	}

	fn cause(&self) -> Option<&dyn error::Error> {
		None
	}
}

#[derive(Debug, Clone)]
pub struct RebaseError;

impl fmt::Display for RebaseError {
	fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
		write!(f, "Rebase error")
	}
}

impl error::Error for RebaseError {
	fn description(&self) -> &str {
		"RebaseError"
	}

	fn cause(&self) -> Option<&dyn error::Error> {
		None
	}
}

impl FileRec {
	pub fn from_reader<'b>(reader: &mut dyn Read) -> Result<FileRec, FileRecParseError<'b>> {
		let typ = reader
			.read_u16::<LittleEndian>()
			.map_err(|_| FileRecParseError("Failed to parse file rec typ"))?;
		let extra_data = reader
			.read_u32::<LittleEndian>()
			.map_err(|_| FileRecParseError("Failed to parse file rec extra data"))?;
		let data_size = reader
			.read_u32::<LittleEndian>()
			.map_err(|_| FileRecParseError("Failed to parse file rec data size"))?
			as usize;

		if data_size > 0x8000000 {
			return Err(FileRecParseError("File rec data size too large"));
		}

		let mut data = vec![0; data_size];
		reader
			.read_exact(&mut data)
			.map_err(|_| FileRecParseError("Failed to parse file rec data"))?;

		let typ = UninstallRecTyp::from(typ);

		Ok(FileRec {
			typ,
			extra_data,
			data,
		})
	}

	pub fn to_writer<'b>(&self, writer: &mut dyn Write) -> Result<(), FileRecWriteError<'b>> {
		writer
			.write_u16::<LittleEndian>(self.typ as u16)
			.map_err(|_| FileRecWriteError("Failed to write file rec typ to buffer"))?;

		writer
			.write_u32::<LittleEndian>(self.extra_data)
			.map_err(|_| FileRecWriteError("Failed to write file rec extra data to buffer"))?;

		writer
			.write_u32::<LittleEndian>(self.data.len() as u32)
			.map_err(|_| FileRecWriteError("Failed to write file rec data size to buffer"))?;

		writer
			.write_all(&self.data)
			.map_err(|_| FileRecWriteError("Failed to write file rec data to buffer"))?;

		Ok(())
	}

	pub fn rebase(&self, update_path: &Path) -> Result<FileRec, Box<dyn error::Error>> {
		let paths = decode_strings(&self.data)?;

		let from = update_path.to_str().ok_or(RebaseError)?;
		let to = update_path
			.parent()
			.and_then(|p| p.to_str())
			.ok_or(RebaseError)?;

		let rebased_paths: Vec<String> = paths
			.iter()
			.map(|p| {
				if let Some(p) = p.strip_prefix(from) {
					format!("{to}{p}")
				} else {
					p.clone()
				}
			})
			.collect();

		Ok(FileRec {
			typ: self.typ,
			extra_data: self.extra_data,
			data: encode_strings(&rebased_paths)?,
		})
	}
}
