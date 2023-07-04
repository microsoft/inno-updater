/*-----------------------------------------------------------------------------------------
 *  Copyright (c) Microsoft Corporation. All rights reserved.
 *  Licensed under the MIT License. See LICENSE in the project root for license information.
 *----------------------------------------------------------------------------------------*/

use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use crc::{Crc, CRC_32_ISO_HDLC};
use std::io::prelude::*;
use std::string::String;
use std::{error, fmt};
use strings;

pub const CRC32: Crc<u32> = Crc::<u32>::new(&CRC_32_ISO_HDLC);

#[derive(Debug, Clone)]
pub struct HeaderParseError<'a>(&'a str);

impl<'a> fmt::Display for HeaderParseError<'a> {
	fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
		write!(f, "Header parse error: {}", self.0)
	}
}

impl<'a> error::Error for HeaderParseError<'a> {
	fn description(&self) -> &str {
		"HeaderParseError"
	}

	fn cause(&self) -> Option<&dyn error::Error> {
		None
	}
}

#[derive(Debug, Clone)]
pub struct HeaderWriteError<'a>(&'a str);

impl<'a> fmt::Display for HeaderWriteError<'a> {
	fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
		write!(f, "Header write error: {}", self.0)
	}
}

impl<'a> error::Error for HeaderWriteError<'a> {
	fn description(&self) -> &str {
		"HeaderWriteError"
	}

	fn cause(&self) -> Option<&dyn error::Error> {
		None
	}
}

// HEADER

pub const HEADER_SIZE: usize = 448;
const HEADER_ID_32: &str = "Inno Setup Uninstall Log (b)";
const HEADER_ID_64: &str = "Inno Setup Uninstall Log (b) 64-bit";
const HIGHEST_SUPPORTED_VERSION: i32 = 1048;

#[derive(Clone)]
pub struct Header {
	id: String,       // 64 bytes
	app_id: String,   // 128
	app_name: String, // 128
	version: i32,
	pub num_recs: usize,
	pub end_offset: u32,
	flags: u32,
	crc: u32,
}

impl fmt::Debug for Header {
	fn fmt(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
		write!(
			formatter,
			"Header, id: {}, app id: {}, app name: {}, version: {}, num recs: {}, end offset: {}, flags: 0x{:x}, crc: 0x{:x}",
			self.id,
			self.app_id,
			self.app_name,
			self.version,
			self.num_recs,
			self.end_offset,
			self.flags,
			self.crc,
		)
	}
}

impl Header {
	pub fn from_reader<'a>(reader: &mut dyn Read) -> Result<Header, HeaderParseError<'a>> {
		let mut buf = [0; HEADER_SIZE];
		reader
			.read_exact(&mut buf)
			.map_err(|_| HeaderParseError("Failed to read header to buffer"))?;

		let mut read: &[u8] = &buf;
		let id = strings::read_utf8_string(&mut read, 64)
			.map_err(|_| HeaderParseError("Failed to parse header ID"))?;
		let app_id = strings::read_utf8_string(&mut read, 128)
			.map_err(|_| HeaderParseError("Failed to parse header app ID"))?;
		let app_name = strings::read_utf8_string(&mut read, 128)
			.map_err(|_| HeaderParseError("Failed to parse header app name"))?;
		let version = read
			.read_i32::<LittleEndian>()
			.map_err(|_| HeaderParseError("Failed to parse header version"))?;
		let num_recs = read
			.read_i32::<LittleEndian>()
			.map_err(|_| HeaderParseError("Failed to parse header num recs"))? as usize;
		let end_offset = read
			.read_u32::<LittleEndian>()
			.map_err(|_| HeaderParseError("Failed to parse header end offset"))?;
		let flags = read
			.read_u32::<LittleEndian>()
			.map_err(|_| HeaderParseError("Failed to parse header flags"))?;

		let mut reserved = [0; 108];
		read.read_exact(&mut reserved)
			.map_err(|_| HeaderParseError("Failed to parse header reserved"))?;

		let crc = read
			.read_u32::<LittleEndian>()
			.map_err(|_| HeaderParseError("Failed to parse header crc"))?;

		if CRC32.checksum(&buf[..HEADER_SIZE - 4]) != crc {
			return Err(HeaderParseError("CRC32 check failed"));
		}

		match id.as_ref() {
			HEADER_ID_32 => (),
			HEADER_ID_64 => (),
			_ => return Err(HeaderParseError("Invalid header ID")),
		}

		if version > HIGHEST_SUPPORTED_VERSION {
			return Err(HeaderParseError("Header version not supported"));
		}

		Ok(Header {
			id,
			app_id,
			app_name,
			version,
			num_recs,
			end_offset,
			flags,
			crc,
		})
	}

	pub fn to_writer<'a>(&self, writer: &mut dyn Write) -> Result<(), HeaderWriteError<'a>> {
		let mut buf = [0; HEADER_SIZE];
		{
			let mut buf_writer: &mut [u8] = &mut buf;

			strings::write_utf8_string(&mut buf_writer, &self.id, 64)
				.map_err(|_| HeaderWriteError("Failed to write header id to buffer"))?;
			strings::write_utf8_string(&mut buf_writer, &self.app_id, 128)
				.map_err(|_| HeaderWriteError("Failed to write header app id to buffer"))?;
			strings::write_utf8_string(&mut buf_writer, &self.app_name, 128)
				.map_err(|_| HeaderWriteError("Failed to write header app name to buffer"))?;

			buf_writer
				.write_i32::<LittleEndian>(self.version)
				.map_err(|_| HeaderWriteError("Failed to write header version to buffer"))?;
			buf_writer
				.write_i32::<LittleEndian>(self.num_recs as i32)
				.map_err(|_| HeaderWriteError("Failed to write header num recs to buffer"))?;
			buf_writer
				.write_u32::<LittleEndian>(self.end_offset)
				.map_err(|_| HeaderWriteError("Failed to write header end offset to buffer"))?;
			buf_writer
				.write_u32::<LittleEndian>(self.flags)
				.map_err(|_| HeaderWriteError("Failed to write header flags to buffer"))?;

			let reserved = vec![0; 108];
			buf_writer
				.write_all(&reserved)
				.map_err(|_| HeaderWriteError("Failed to write header reserved to buffer"))?;
		}

		let crc = CRC32.checksum(&buf[..HEADER_SIZE - 4]);

		{
			let mut buf_writer = &mut buf[HEADER_SIZE - 4..];

			buf_writer
				.write_u32::<LittleEndian>(crc)
				.map_err(|_| HeaderWriteError("Failed to write header crc to buffer"))?;
		}

		writer
			.write_all(&buf)
			.map_err(|_| HeaderWriteError("Failed to write header to writer"))?;

		Ok(())
	}
}
