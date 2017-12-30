extern crate byteorder;
extern crate crc;

use std::io;
use std::io::prelude::*;
use std::fmt;
use std::fs;
use std::vec::Vec;
use std::string;
use std::collections::HashMap;
use byteorder::{LittleEndian, ReadBytesExt};
use crc::{crc32, Hasher32};

#[derive(Debug)]
enum ReadUtf8StringError {
	IOError(io::Error),
	UTF8Error(string::FromUtf8Error),
}

fn read_utf8_string(reader: &mut Read, capacity: usize) -> Result<String, ReadUtf8StringError> {
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

const BLOCK_MAX_SIZE: usize = 4096;

struct BlockRead<'a> {
	reader: &'a mut Read,
	buffer: [u8; 4096],
	pos: usize,
	left: usize,
}

impl<'a> BlockRead<'a> {
	fn new(reader: &'a mut Read) -> BlockRead<'a> {
		BlockRead { reader, buffer: [0; 4096], pos: 0, left: 0 }
	}

	fn fill_buffer(&mut self) -> Result<(), io::Error> {
		let size = self.reader.read_u32::<LittleEndian>()?;
		let not_size = self.reader.read_u32::<LittleEndian>()?;
		let crc = self.reader.read_u32::<LittleEndian>()?;

		if size != !not_size {
			return Err(io::Error::new(io::ErrorKind::InvalidData, "block header size is corrupt"));
		}

		if size > BLOCK_MAX_SIZE as u32 {
			return Err(io::Error::new(io::ErrorKind::InvalidData, "block header size is too large"));
		}

		let size = size as usize;
		let mut buffer = &mut self.buffer[..size];
		self.reader.read_exact(&mut buffer)?;

		let mut digest = crc32::Digest::new(crc32::IEEE);
		digest.write(buffer);
		let actual_crc = digest.sum32();

		if actual_crc != crc {
			return Err(io::Error::new(io::ErrorKind::InvalidData, "block header crc32 check failed"));
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

			let to = &mut buf[p..p+s];
			let from = &self.buffer[self.pos..self.pos+s];

			to.copy_from_slice(from);
			self.pos += s;
			self.left -= s;
			p += s;
			size -= s;
		}

		Ok(buf.len())
	}
}

// FILE REC

#[derive(Copy, Clone)]
enum UninstallRecTyp {
	UserDefined           = 0x01,
	StartInstall          = 0x10,
	EndInstall            = 0x11,
	CompiledCode          = 0x20,
	Run                   = 0x80,
	DeleteDirOrFiles      = 0x81,
	DeleteFile            = 0x82,
	DeleteGroupOrItem     = 0x83,
	IniDeleteEntry        = 0x84,
	IniDeleteSection      = 0x85,
	RegDeleteEntireKey    = 0x86,
	RegClearValue         = 0x87,
	RegDeleteKeyIfEmpty   = 0x88,
	RegDeleteValue        = 0x89,
	DecrementSharedCount  = 0x8A,
	RefreshFileAssoc      = 0x8B,
	MutexCheck            = 0x8C,
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

struct FileRec {
	typ: UninstallRecTyp,
	extra_data: u32,
	data: Vec<u8>,
}

impl<'a> std::fmt::Debug for FileRec {
	fn fmt(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
		write!(formatter,
			"FileRec 0x{:x} 0x{:x} {} bytes",
			self.typ as u32,
			self.extra_data as u32,
			self.data.len(),
		)
	}
}

impl<'a> FileRec {
	fn from_reader(reader: &mut Read) -> FileRec {
		let typ = reader.read_u16::<LittleEndian>().expect("file rec typ");
		let extra_data = reader.read_u32::<LittleEndian>().expect("file rec extra data");
		let data_size = reader.read_u32::<LittleEndian>().expect("file rec data size") as usize;

		if data_size > 0x8000000 {
			panic!("file rec data size too large {}", data_size);
		}

		let mut data = vec![0; data_size];
		reader.read_exact(&mut data).expect("file rec data");

		let typ = UninstallRecTyp::from(typ);

		FileRec {
			typ,
			extra_data,
			data,
		}
	}
}

// HEADER

const HEADER_SIZE: usize = 448;
const HEADER_ID_32: &str = "Inno Setup Uninstall Log (b)";
const HEADER_ID_64: &str = "Inno Setup Uninstall Log (b) 64-bit";
const HIGHEST_SUPPORTED_VERSION: i32 = 1048;

struct Header {
	id: String,       // 64 bytes
	app_id: String,   // 128
	app_name: String, // 128
	version: i32,
	num_recs: usize,
	end_offset: u32,
	flags: u32,
	crc: u32,
}

impl std::fmt::Debug for Header {
	fn fmt(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
		write!(formatter,
			"Header\nid: {}\napp id: {}\napp name: {}\nversion: {}\nnum recs: {}\nend offset: {}\nflags: 0x{:x}\ncrc: 0x{:x}",
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
	fn from_reader(reader: &mut Read) -> Header {
		let mut buf = [0; HEADER_SIZE];
		reader.read_exact(&mut buf).expect("read error");
		let mut read: &[u8] = &buf;

		let id = read_utf8_string(&mut read, 64).expect("header id");
		let app_id = read_utf8_string(&mut read, 128).expect("header app id");
		let app_name = read_utf8_string(&mut read, 128).expect("header app name");
		let version = read.read_i32::<LittleEndian>().expect("header version");
		let num_recs = read.read_i32::<LittleEndian>().expect("header num recs") as usize;
		let end_offset = read
			.read_u32::<LittleEndian>()
			.expect("header end offset");
		let flags = read.read_u32::<LittleEndian>().expect("header flags");

		let mut reserved = [0; 108];
		read.read_exact(&mut reserved).expect("header reserved");
		let crc = read.read_u32::<LittleEndian>().expect("header crc");

		let mut digest = crc32::Digest::new(crc32::IEEE);
		digest.write(&buf[..HEADER_SIZE-4]);
		let actual_crc = digest.sum32();

		if actual_crc != crc {
			panic!("header crc32 check failed");
		}

		match id.as_ref() {
			HEADER_ID_32 => (),
			HEADER_ID_64 => (),
			_ => panic!("header id not valid")
		}

		if version > HIGHEST_SUPPORTED_VERSION {
			panic!("header version not supported");
		}

		Header {
			id,
			app_id,
			app_name,
			version,
			num_recs,
			end_offset,
			flags,
			crc,
		}
	}
}

// MAIN

fn print_statistics(recs: &[FileRec]) {
	let mut map: HashMap<u16,u32> = HashMap::new();

	for rec in recs {
		let count = map.entry(rec.typ as u16).or_insert(0);
    *count += 1;
	}

	for (k, c) in &map {
		println!("records 0x{:x} {}", k, c);
	}
}

fn main() {
	let filename = "unins000.dat";
	let mut f = fs::File::open(filename).expect("file not found");

	let header = Header::from_reader(&mut f);
	let mut reader = BlockRead::new(&mut f);
	let mut recs = Vec::with_capacity(header.num_recs);

	for _ in 0..header.num_recs {
		recs.push(FileRec::from_reader(&mut reader));
	}

	// println!("{:?}", header);
	print_statistics(&recs);
}
