extern crate byteorder;
extern crate crc;

mod blockio;
mod strings;
mod model;

use std::fs;
use std::vec::Vec;
use std::collections::HashMap;

// MAIN

fn print_statistics(recs: &[model::FileRec]) {
	let mut map: HashMap<u16, u32> = HashMap::new();

	for rec in recs {
		let count = map.entry(rec.typ as u16).or_insert(0);
		*count += 1;
	}

	println!("Statistics");

	for (k, c) in &map {
		println!("records 0x{:x} {}", k, c);
	}
}

fn main() {
	let filename = "unins000.dat";
	let mut f = fs::File::open(filename).expect("file not found");

	let header = model::Header::from_reader(&mut f);
	let mut reader = blockio::BlockRead::new(&mut f);
	let mut recs = Vec::with_capacity(header.num_recs);

	for _ in 0..header.num_recs {
		let mut rec = model::FileRec::from_reader(&mut reader);

		match rec.typ {
			model::UninstallRecTyp::DeleteDirOrFiles | model::UninstallRecTyp::DeleteFile => rec.rebase(
				"C:\\Program Files (x86)\\ProcMon\\update",
				"C:\\Program Files (x86)\\ProcMon",
			),
			_ => (),
		}

		recs.push(rec);
	}

	println!("{:?}", header);
	print_statistics(&recs);
}
