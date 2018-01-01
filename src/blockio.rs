use std::io;
use std::io::prelude::*;
use byteorder::{LittleEndian, ReadBytesExt};
use crc::{Hasher32, crc32};

const BLOCK_MAX_SIZE: usize = 4096;

pub struct BlockRead<'a> {
	reader: &'a mut Read,
	buffer: [u8; 4096],
	pos: usize,
	left: usize,
}

impl<'a> BlockRead<'a> {
	pub fn new(reader: &'a mut Read) -> BlockRead<'a> {
		BlockRead {
			reader,
			buffer: [0; 4096],
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
				"block header size is corrupt",
			));
		}

		if size > BLOCK_MAX_SIZE as u32 {
			return Err(io::Error::new(
				io::ErrorKind::InvalidData,
				"block header size is too large",
			));
		}

		let size = size as usize;
		let mut buffer = &mut self.buffer[..size];
		self.reader.read_exact(&mut buffer)?;

		let mut digest = crc32::Digest::new(crc32::IEEE);
		digest.write(buffer);
		let actual_crc = digest.sum32();

		if actual_crc != crc {
			return Err(io::Error::new(
				io::ErrorKind::InvalidData,
				"block header crc32 check failed",
			));
		}

		self.pos = 0;
		self.left = size;

		Ok(())
	}
}

impl<'a> Read for BlockRead<'a> {
	fn read(&mut self, buf: &mut [u8]) -> Result<usize, io::Error> {
		let mut p: usize = 0;
		let mut size = buf.len();

		while size > 0 {
			if self.left == 0 {
				self.fill_buffer()?;
			}

			let mut s = size;

			if s > self.left {
				s = self.left;
			}

			let to = &mut buf[p..p + s];
			let from = &self.buffer[self.pos..self.pos + s];

			to.copy_from_slice(from);
			self.pos += s;
			self.left -= s;
			p += s;
			size -= s;
		}

		Ok(buf.len())
	}
}
