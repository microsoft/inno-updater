extern crate byteorder;
extern crate crc;

mod blockio;
mod strings;
mod model;

use std::fs;
use std::io;
use std::io::prelude::*;
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
	let input_file = fs::File::open("unins000.dat").expect("file not found");
	let mut input = io::BufReader::new(input_file);

	let header = model::Header::from_reader(&mut input);
	let mut reader = blockio::BlockRead::new(&mut input);
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

	let output_file = fs::File::create("output.dat").expect("could not create file");
	let mut output = io::BufWriter::new(output_file);

	header.to_writer(&mut output);

	let mut writer = blockio::BlockWrite::new(&mut output);

	for rec in recs {
		rec.to_writer(&mut writer);
	}

	writer.flush().expect("flush");
	// println!("{:?}", header);
 // print_statistics(&recs);
}
