extern crate byteorder;

use std::io;
use std::fmt;
use std::fs::File;
use std::string;
use std::io::prelude::*;
use byteorder::{LittleEndian, ReadBytesExt};

struct Header {
	id: String,       // 64 bytes
	app_id: String,   // 128
	app_name: String, // 128
	version: i32,
	num_recs: i32,
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

impl Header {
	fn deserialize(reader: &mut Read) -> Header {
		// let mut buffer = [0; 448];
	// reader.read_exact(&mut buffer).expect("header");

		// let iterator = (&buffer[..]).iter();
	// let foo = iterator.take(64);


		// let bar = foo.iter();


		// let r = &buffer[..];

		// r.iter();
	// let l = r.read_exact(1);

		let id = read_utf8_string(reader, 64).expect("header id");
		let app_id = read_utf8_string(reader, 128).expect("header app id");
		let app_name = read_utf8_string(reader, 128).expect("header app name");
		let version = reader.read_i32::<LittleEndian>().expect("header version");
		let num_recs = reader.read_i32::<LittleEndian>().expect("header num recs");
		let end_offset = reader
			.read_u32::<LittleEndian>()
			.expect("header end offset");
		let flags = reader.read_u32::<LittleEndian>().expect("header flags");

		let mut reserved = [0; 108];
		reader.read_exact(&mut reserved).expect("header reserved");
		let crc = reader.read_u32::<LittleEndian>().expect("header crc");

		Header {
			id,
			app_id,
			app_name,
			version,
			num_recs,
			end_offset,
			flags,
			// reserved: Reserved { data: reserved },
			crc,
		}
	}
}

// const HEADER_ID_32: &str = "Inno Setup Uninstall Log (b)";
// const HEADER_ID_64: &str = "Inno Setup Uninstall Log (b) 64-bit";
// const HIGHEST_SUPPORTED_VERSION: i32 = 1048;

fn main() {
	let filename = "unins000.dat";
	let mut f = File::open(filename).expect("file not found");
	let header = Header::deserialize(&mut f);


	// let mut contents = String::new();
 // f.read_to_string(&mut contents)
 // 	.expect("something went wrong reading the file");

	println!("{:?}", header);
}
