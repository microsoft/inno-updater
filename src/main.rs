/*-----------------------------------------------------------------------------------------
 *  Copyright (c) Microsoft Corporation. All rights reserved.
 *  Licensed under the MIT License. See LICENSE in the project root for license information.
 *----------------------------------------------------------------------------------------*/

#![windows_subsystem = "windows"]

extern crate byteorder;
extern crate crc;
extern crate winapi;

mod blockio;
mod strings;
mod model;
mod gui;

use std::{env, fs, io, panic, thread, time};
use std::path::{Path, PathBuf};
use std::io::prelude::*;
use std::vec::Vec;
use model::{FileRec, Header};

// MAIN

// fn print_statistics(recs: &[FileRec]) {
// 	use std::collections::HashMap;
// 	let mut map: HashMap<u16, u32> = HashMap::new();

// 	for rec in recs {
// 		let count = map.entry(rec.typ as u16).or_insert(0);
// 		*count += 1;
// 	}

// 	println!("Statistics");

// 	for (k, c) in &map {
// 		println!("records 0x{:x} {}", k, c);
// 	}
// }

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

/**
 * Quadratic backoff retry mechanism
 */
fn retry<F, R, E>(closure: F) -> Result<R, E>
where
	F: Fn() -> Result<R, E>,
{
	let mut attempt: u64 = 0;

	loop {
		attempt += 1;

		let result = closure();
		match result {
			Ok(_) => return result,
			Err(_) => {
				if attempt > 10 {
					return result;
				}

				thread::sleep(time::Duration::from_millis(attempt.pow(2) * 50));
				return result;
			}
		}
	}
}

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

	// get the current exe name
	let exe_path = env::current_exe()?;
	let exe_name = exe_path
		.file_name()
		.unwrap()
		.to_str()
		.ok_or(io::Error::new(io::ErrorKind::Other, "oh no!"))?;

	// delete all current files
	for entry in fs::read_dir(&root_path)? {
		let entry = entry?;
		let entry_name = entry.file_name();
		let entry_name = entry_name
			.to_str()
			.ok_or(io::Error::new(io::ErrorKind::Other, "oh no!"))?;

		// don't delete the update folder
		if entry_name == update_folder_name {
			continue;
		}

		// don't delete any of the unins* files
		if String::from(entry_name).starts_with("unins") {
			continue;
		}

		// don't delete ourselves
		if entry_name == exe_name {
			continue;
		}

		// attempt to delete
		retry(|| {
			let entry_file_type = entry.file_type()?;
			let entry_path = entry.path();

			if entry_file_type.is_file() {
				fs::remove_file(&entry_path)?;
			} else {
				fs::remove_dir_all(&entry_path)?;
			}

			if !entry_path.exists() {
				Ok(())
			} else {
				Err(io::Error::new(io::ErrorKind::Other, "path still exists"))
			}
		})?
	}

	// move update to current
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

	fs::remove_dir_all(update_path)
}

fn do_update(uninstdat_path: PathBuf, update_folder_name: String) {
	let (header, recs) = read_file(&uninstdat_path);

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

fn update(uninstdat_path: PathBuf, update_folder_name: String, silent: bool) {
	if silent {
		// wait a bit before starting
		thread::sleep(time::Duration::from_secs(1));
		do_update(uninstdat_path, update_folder_name);
	} else {
		let window = gui::create_progress_window();

		thread::spawn(move || {
			// wait a bit before starting
			thread::sleep(time::Duration::from_secs(1));

			panic::catch_unwind(|| do_update(uninstdat_path, update_folder_name)).ok();
			window.exit();
		});

		gui::event_loop();
	}
}

fn main() {
	let args: Vec<String> = env::args().filter(|a| !a.starts_with("--")).collect();

	if args.len() < 4 {
		println!("Usage: inno_updater.exe update_folder_name app_path silent");
		std::process::exit(1);
	}

	let update_folder_name = args[1].clone();
	let uninstdat_path = PathBuf::from(&args[2]);
	let silent = args[3].clone();

	if !uninstdat_path.is_absolute() {
		println!("Path needs to be absolute");
		std::process::exit(1);
	}

	if silent != "true" && silent != "false" {
		println!("Silent needs to be true or false");
		std::process::exit(1);
	}

	update(uninstdat_path, update_folder_name, silent == "true");
}

// fn main() {
// 	let window = gui::create_progress_window();

// 	thread::spawn(move || {
// 		thread::sleep(time::Duration::from_secs(5));
// 		window.exit();
// 	});

// 	gui::event_loop();
// }
