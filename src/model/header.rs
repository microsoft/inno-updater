use std::fmt;
use strings;
use std::string::String;
use std::io::prelude::*;
use byteorder::{LittleEndian, ReadBytesExt};
use crc::{Hasher32, crc32};

// HEADER

const HEADER_SIZE: usize = 448;
const HEADER_ID_32: &str = "Inno Setup Uninstall Log (b)";
const HEADER_ID_64: &str = "Inno Setup Uninstall Log (b) 64-bit";
const HIGHEST_SUPPORTED_VERSION: i32 = 1048;

pub struct Header {
	id: String,       // 64 bytes
	app_id: String,   // 128
	app_name: String, // 128
	version: i32,
	pub num_recs: usize,
	end_offset: u32,
	flags: u32,
	crc: u32,
}

impl fmt::Debug for Header {
	fn fmt(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
		write!(
			formatter,
			"Header
id: {}
app id: {}
app name: {}
version: {}
num recs: {}
end offset: {}
flags: 0x{:x}
crc: 0x{:x}",
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
	pub fn from_reader(reader: &mut Read) -> Header {
		let mut buf = [0; HEADER_SIZE];
		reader.read_exact(&mut buf).expect("read error");
		let mut read: &[u8] = &buf;

		let id = strings::read_utf8_string(&mut read, 64).expect("header id");
		let app_id = strings::read_utf8_string(&mut read, 128).expect("header app id");
		let app_name = strings::read_utf8_string(&mut read, 128).expect("header app name");
		let version = read.read_i32::<LittleEndian>().expect("header version");
		let num_recs = read.read_i32::<LittleEndian>().expect("header num recs") as usize;
		let end_offset = read.read_u32::<LittleEndian>().expect("header end offset");
		let flags = read.read_u32::<LittleEndian>().expect("header flags");

		let mut reserved = [0; 108];
		read.read_exact(&mut reserved).expect("header reserved");
		let crc = read.read_u32::<LittleEndian>().expect("header crc");

		let mut digest = crc32::Digest::new(crc32::IEEE);
		digest.write(&buf[..HEADER_SIZE - 4]);
		let actual_crc = digest.sum32();

		if actual_crc != crc {
			panic!("header crc32 check failed");
		}

		match id.as_ref() {
			HEADER_ID_32 => (),
			HEADER_ID_64 => (),
			_ => panic!("header id not valid"),
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
