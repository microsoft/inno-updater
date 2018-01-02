use std::fmt;
use std::string::String;
use std::io::prelude::*;
use byteorder::{ByteOrder, LittleEndian, ReadBytesExt, WriteBytesExt};

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

pub struct FileRec {
	pub typ: UninstallRecTyp,
	extra_data: u32,
	data: Vec<u8>,
}

impl<'a> fmt::Debug for FileRec {
	fn fmt(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
		write!(
			formatter,
			"FileRec 0x{:x} 0x{:x} {} bytes",
			self.typ as u32,
			self.extra_data as u32,
			self.data.len(),
		)
	}
}

fn decode_strings(data: &[u8]) -> Vec<String> {
	let mut result: Vec<String> = Vec::with_capacity(10);
	let mut slice = data.clone();

	loop {
		let reader: &mut Read = &mut slice.clone();
		let byte_result = reader.read_u8().expect("file rec string header");

		match byte_result {
			0x00...0xfc => panic!("what 0x{:x}", byte_result),
			0xfd => panic!("what 0x{:x}", byte_result),
			0xfe => {
				let size = reader
					.read_i32::<LittleEndian>()
					.expect("file rec string size");

				let size = -size as usize;

				if size > 0 {
					assert!(size % 2 == 0);

					let mut u16data: Vec<u16> = vec![0; size / 2];
					LittleEndian::read_u16_into(&slice[5..5 + size], &mut u16data);

					let string = String::from_utf16(&u16data).expect("file rec data string");
					result.push(string);
				}

				slice = &slice[5 + size..];
			}
			0xff => {
				assert!(slice.len() == 1);
				return result;
			}
			_ => panic!("invalid file rec string header"),
		}
	}
}

impl<'a> FileRec {
	pub fn from_reader(reader: &mut Read) -> FileRec {
		let typ = reader.read_u16::<LittleEndian>().expect("file rec typ");
		let extra_data = reader
			.read_u32::<LittleEndian>()
			.expect("file rec extra data");
		let data_size = reader
			.read_u32::<LittleEndian>()
			.expect("file rec data size") as usize;

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

	pub fn rebase(&mut self, from: &str, to: &str) {
		let strings = decode_strings(&self.data);

		println!("{}", strings.len());

		// let (mut path, old_size) = self.get_string();

		// if path.starts_with(from) {
	// 	path = [to, &path[from.len()..]].join("");
	// }

		// let u16data: Vec<u16> = path.encode_utf16().collect();
	// let new_size = u16data.len() * 2;
	// let mut data: Vec<u8> = vec![0; self.data.len() - old_size + new_size];

		// {
	// 	let mut slice: &mut [u8] = &mut data[..];
	// 	let writer: &mut Write = &mut slice;

		// 	writer.write_u8(0xfe).expect("file rec data first byte");
	// 	writer
	// 		.write_i32::<LittleEndian>(-(new_size as i32))
	// 		.expect("file rec data size");
	// }

		// {
	// 	let slice = &mut data[5..5 + new_size];
	// 	LittleEndian::write_u16_into(&u16data, slice);
	// }

		// {
	// 	let old_rest = &self.data[5 + old_size..];
	// 	let new_rest = &mut data[5 + new_size..];
	// 	new_rest.copy_from_slice(old_rest);
	// }

		// self.data = data;
	}
}
