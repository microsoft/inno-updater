extern crate byteorder;
extern crate clap;
extern crate crc;

mod blockio;
mod strings;
mod model;

use std::fs;
use std::path::{Path, PathBuf};
use std::io;
use std::io::prelude::*;
use std::vec::Vec;
use std::collections::HashMap;
use clap::{App, Arg, SubCommand};
use model::{FileRec, Header};

// MAIN

fn print_statistics(recs: &[FileRec]) {
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

fn read_file(path: &Path) -> (Header, Vec<FileRec>) {
	let input_file = fs::File::open(path).expect("file not found");
	let mut input = io::BufReader::new(input_file);

	let header = Header::from_reader(&mut input);
	let mut reader = blockio::BlockRead::new(&mut input);
	let mut recs = Vec::with_capacity(header.num_recs);

	for _ in 0..header.num_recs {
		recs.push(FileRec::from_reader(&mut reader));
	}

	(header, recs)
}

fn main() {
	let app = App::new("VSCode Update Helper Tool")
		.version("1.0")
		.author("Microsoft")
		.arg(
			Arg::with_name("apply-update")
				.long("apply-update")
				.help("Applies an update")
				.value_name("FOLDER_NAME")
				.takes_value(true),
		)
		.arg(
			Arg::with_name("INPUT")
				.help("Input file")
				.required(true)
				.index(1),
		);

	let m = app.get_matches();
	let input_file_path = m.value_of("INPUT").unwrap();

	let update_folder_name = m.value_of("apply-update");

	// println!("{}", input);

	// match m.subcommand() {
	// 	("apply-update", Some(m)) => {
	// 		println!("applying");
	// 	}
	// 	_ => {
	// 		panic!("oh no");
	// 	}
	// }

	// if let Some(matches) = matches.subcommand_matches("apply-update") {
	// 	println!("{:?}", matches);
	// } else {
	// 	app.print_help();
	// }

	// let matches = clap_app!(myapp =>
	// 	(version: "1.0")
	// 	(author: "Microsoft")
	// 	(about: "Update helper tool")
	// 	(@subcommand apply-update =>
	// 		(about: "Applies update to Inno Setup data file")
	// 	)
	// ).get_matches();

	let uninstdat_path = std::fs::canonicalize(input_file_path).expect("uninstdat path");
	let (header, recs) = read_file(&uninstdat_path);

	match update_folder_name {
		Some(name) => {
			let root_path = uninstdat_path.parent().expect("parent");
			let mut update_path = PathBuf::from(root_path);
			update_path.push(name);

			println!("uninstdat: {:?}", uninstdat_path);
			println!("update_path: {:?}", update_path);
			println!("{}", name);
		}
		_ => {
			println!("{:?}", header);
			print_statistics(&recs);
		}
	};

	// match rec.typ {
	// 	model::UninstallRecTyp::DeleteDirOrFiles | model::UninstallRecTyp::DeleteFile => rec.rebase(
	// 		"C:\\Program Files (x86)\\ProcMon\\update",
	// 		"C:\\Program Files (x86)\\ProcMon",
	// 	),
	// 	_ => (),
	// }

	// let output_file = fs::File::create("output.dat").expect("could not create file");
	// let mut output = io::BufWriter::new(output_file);

	// header.to_writer(&mut output);

	// let mut writer = blockio::BlockWrite::new(&mut output);

	// for rec in recs {
	// 	rec.to_writer(&mut writer);
	// }

	// writer.flush().expect("flush");
	// println!("{:?}", header);
}
