// #![windows_subsystem = "windows"]

extern crate byteorder;
extern crate clap;
extern crate crc;
extern crate winapi;

mod blockio;
mod strings;
mod model;
mod gui;

use std::{env, fs, io, thread};
use std::path::{Path, PathBuf};
use std::io::prelude::*;
use std::vec::Vec;
use std::collections::HashMap;
use clap::{App, Arg};
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

fn write_file(path: &Path, header: &Header, recs: Vec<FileRec>) {
	let mut output_file = fs::File::create(path).expect("could not create file");

	// skip header
	output_file.seek(io::SeekFrom::Start(448)).expect("seek");

	{
		let mut output = io::BufWriter::new(&output_file);
		let mut writer = blockio::BlockWrite::new(&mut output);

		for rec in recs {
			rec.to_writer(&mut writer);
		}

		writer.flush().expect("flush");
	}

	let mut header = header.clone();

	// what's the full file size?
	let end_offset = output_file.seek(io::SeekFrom::Current(0)).unwrap();
	header.end_offset = end_offset as u32;

	// go back to beginning
	output_file.seek(io::SeekFrom::Start(0)).unwrap();

	let mut output = io::BufWriter::new(&output_file);
	header.to_writer(&mut output);

	output.flush().expect("flush");
}

const OLD_NAME: &str = "old";

fn move_update(uninstdat_path: &Path, update_folder_name: &str) -> Result<(), io::Error> {
	let root_path = uninstdat_path.parent().expect("parent");

	let mut update_path = PathBuf::from(root_path);
	update_path.push(update_folder_name);

	let stat = fs::metadata(&update_path)?;

	if !stat.is_dir() {
		return Err(io::Error::new(
			io::ErrorKind::Other,
			"Update folder is not a directory",
		));
	}

	let mut old_path = PathBuf::from(root_path);
	old_path.push(OLD_NAME);

	// make sure `old` is an empty directory
	fs::remove_dir_all(&old_path).ok();
	fs::create_dir(&old_path)?;

	// get the current exe name
	let exe_path = env::current_exe()?;
	let exe_name = exe_path
		.file_name()
		.unwrap()
		.to_str()
		.ok_or(io::Error::new(io::ErrorKind::Other, "oh no!"))?;

	// move all current files to `old`
	for entry in fs::read_dir(&root_path)? {
		let entry = entry?;
		let entry_name = entry.file_name();
		let entry_name = entry_name
			.to_str()
			.ok_or(io::Error::new(io::ErrorKind::Other, "oh no!"))?;

		// don't move the update folder nor the `old` folder
		if entry_name == update_folder_name || entry_name == OLD_NAME {
			continue;
		}

		// don't move any of the unins* files
		if String::from(entry_name).starts_with("unins") {
			continue;
		}

		// don't move ourselves
		if entry_name == exe_name {
			continue;
		}

		let mut target = old_path.clone();
		target.push(entry_name);
		fs::rename(entry.path(), target)?;
	}

	for entry in fs::read_dir(&update_path)? {
		let entry = entry?;
		let entry_name = entry.file_name();
		let entry_name = entry_name
			.to_str()
			.ok_or(io::Error::new(io::ErrorKind::Other, "oh no!"))?;

		let mut target = PathBuf::from(root_path);
		target.push(entry_name);
		fs::rename(entry.path(), target)?;
	}

	fs::remove_dir_all(update_path)?;
	fs::remove_dir_all(old_path)
}

fn do_update(
	uninstdat_path: PathBuf,
	update_folder_name: String,
	header: Header,
	recs: Vec<FileRec>,
) {
	let root_path = uninstdat_path.parent().expect("parent");
	let mut update_path = PathBuf::from(root_path);
	update_path.push(&update_folder_name);

	if let Err(err) = move_update(&uninstdat_path, &update_folder_name) {
		println!("Failed to apply update: {:?}", err);
		return;
	}

	let recs: Vec<FileRec> = recs
		.iter()
		.map(|rec| match rec.typ {
			model::UninstallRecTyp::DeleteDirOrFiles | model::UninstallRecTyp::DeleteFile => {
				rec.rebase(&update_path)
			}
			_ => rec.clone(),
		})
		.collect();

	write_file(&uninstdat_path, &header, recs);
}

fn update(uninstdat_path: PathBuf, update_folder_name: String, header: Header, recs: Vec<FileRec>) {
	let window = gui::create_progress_window();

	thread::spawn(move || {
		do_update(uninstdat_path, update_folder_name, header, recs);
		window.exit();
	});

	gui::event_loop();
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
	let uninstdat_path = Path::new(m.value_of("INPUT").unwrap());

	if !uninstdat_path.is_absolute() {
		println!("Path needs to be absolute");
		std::process::exit(1);
	}

	let (header, recs) = read_file(&uninstdat_path);

	match m.value_of("apply-update") {
		Some(name) => update(
			PathBuf::from(uninstdat_path),
			String::from(name),
			header,
			recs,
		),
		_ => {
			println!("{:?}", header);
			print_statistics(&recs);
		}
	};
}
