/*-----------------------------------------------------------------------------------------
 *  Copyright (c) Microsoft Corporation. All rights reserved.
 *  Licensed under the MIT License. See LICENSE in the project root for license information.
 *----------------------------------------------------------------------------------------*/

use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use crc::{crc32, Hasher32};
use std::io::prelude::*;
use std::{cmp, io};

const BLOCK_MAX_SIZE: usize = 4096;

pub struct BlockRead<'a> {
	reader: &'a mut dyn Read,
	buffer: [u8; BLOCK_MAX_SIZE],
	pos: usize,
	left: usize,
}

impl<'a> BlockRead<'a> {
	pub fn new(reader: &'a mut dyn Read) -> BlockRead<'a> {
		BlockRead {
			reader,
			buffer: [0; BLOCK_MAX_SIZE],
			pos: 0,
			left: 0,
		}
	}

	fn fill_buffer(&mut self) -> Result<(), io::Error> {
		let size = self.reader.read_u32::<LittleEndian>()?;
		let not_size = self.reader.read_u32::<LittleEndian>()?;
		let crc = self.reader.read_u32::<LittleEndian>()?;

		if size != !not_size {
			return Err(io::Error::new(
				io::ErrorKind::InvalidData,
				"Block header size is corrupt",
			));
		}

		if size > BLOCK_MAX_SIZE as u32 {
			return Err(io::Error::new(
				io::ErrorKind::InvalidData,
				"Block header size is too large",
			));
		}

		let size = size as usize;
		let buffer = &mut self.buffer[..size];
		self.reader.read_exact(buffer)?;

		let mut digest = crc32::Digest::new(crc32::IEEE);
		digest.write(buffer);
		let actual_crc = digest.sum32();

		if actual_crc != crc {
			return Err(io::Error::new(
				io::ErrorKind::InvalidData,
				"Block header crc32 check failed",
			));
		}

		self.pos = 0;
		self.left = size;

		Ok(())
	}
}

impl<'a> Read for BlockRead<'a> {
	fn read(&mut self, buf: &mut [u8]) -> Result<usize, io::Error> {
		let mut bytes_read: usize = 0;
		let mut size = buf.len();

		while size > 0 {
			if self.left == 0 {
				self.fill_buffer()?;
			}

			let count = cmp::min(size, self.left);
			let to = &mut buf[bytes_read..bytes_read + count];
			let from = &self.buffer[self.pos..self.pos + count];

			to.copy_from_slice(from);
			self.pos += count;
			self.left -= count;
			bytes_read += count;
			size -= count;
		}

		Ok(buf.len())
	}
}

pub struct BlockWrite<'a> {
	writer: &'a mut dyn Write,
	buffer: [u8; BLOCK_MAX_SIZE],
	pos: usize,
}

impl<'a> BlockWrite<'a> {
	pub fn new(writer: &'a mut dyn Write) -> BlockWrite<'a> {
		BlockWrite {
			writer,
			buffer: [0; BLOCK_MAX_SIZE],
			pos: 0,
		}
	}

	fn flush_buffer(&mut self) -> Result<(), io::Error> {
		if self.pos == 0 {
			return Ok(());
		}

		self.writer.write_u32::<LittleEndian>(self.pos as u32)?;
		self.writer.write_u32::<LittleEndian>(!(self.pos as u32))?;

		let slice = &self.buffer[..self.pos];
		let mut digest = crc32::Digest::new(crc32::IEEE);
		digest.write(slice);

		let crc = digest.sum32();
		self.writer.write_u32::<LittleEndian>(crc)?;
		self.writer.write_all(slice)?;

		self.pos = 0;

		Ok(())
	}
}

impl<'a> Write for BlockWrite<'a> {
	fn write(&mut self, buf: &[u8]) -> Result<usize, io::Error> {
		let mut bytes_written: usize = 0;
		let mut size = buf.len();

		while size > 0 {
			let left = BLOCK_MAX_SIZE - self.pos;
			let count = cmp::min(size, left);

			{
				let to = &mut self.buffer[self.pos..self.pos + count];
				let from = &buf[bytes_written..bytes_written + count];

				to.copy_from_slice(from);
			}

			self.pos += count;
			bytes_written += count;
			size -= count;

			if self.pos == BLOCK_MAX_SIZE {
				self.flush_buffer()?;
			}
		}

		Ok(buf.len())
	}

	fn flush(&mut self) -> Result<(), io::Error> {
		self.flush_buffer()?;
		self.writer.flush()
	}
}
