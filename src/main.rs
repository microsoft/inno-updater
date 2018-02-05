/*-----------------------------------------------------------------------------------------
 *  Copyright (c) Microsoft Corporation. All rights reserved.
 *  Licensed under the MIT License. See LICENSE in the project root for license information.
 *----------------------------------------------------------------------------------------*/

#![windows_subsystem = "windows"]

extern crate byteorder;
extern crate crc;
#[macro_use]
extern crate slog;
extern crate slog_async;
extern crate slog_term;
extern crate winapi;

mod blockio;
mod strings;
mod model;
mod gui;

use std::{env, fs, io, thread, time};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::io::prelude::*;
use std::vec::Vec;
use slog::Drain;
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

fn read_file(path: &Path) -> Result<(Header, Vec<FileRec>), io::Error> {
	let input_file = fs::File::open(path)?;
	let mut input = io::BufReader::new(input_file);

	let header = Header::from_reader(&mut input);
	let mut reader = blockio::BlockRead::new(&mut input);
	let mut recs = Vec::with_capacity(header.num_recs);

	for _ in 0..header.num_recs {
		recs.push(FileRec::from_reader(&mut reader));
	}

	Ok((header, recs))
}

fn write_file(path: &Path, header: &Header, recs: Vec<FileRec>) -> Result<(), io::Error> {
	let mut output_file = fs::File::create(path)?;

	// skip header
	output_file.seek(io::SeekFrom::Start(448))?;

	{
		let mut output = io::BufWriter::new(&output_file);
		let mut writer = blockio::BlockWrite::new(&mut output);

		for rec in recs {
			rec.to_writer(&mut writer);
		}

		writer.flush()?;
	}

	let mut header = header.clone();

	// what's the full file size?
	let end_offset = output_file.seek(io::SeekFrom::Current(0))?;
	header.end_offset = end_offset as u32;

	// go back to beginning
	output_file.seek(io::SeekFrom::Start(0))?;

	let mut output = io::BufWriter::new(&output_file);
	header.to_writer(&mut output);

	output.flush()
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
			}
		}
	}
}

fn move_update(
	log: &slog::Logger,
	uninstdat_path: &Path,
	update_folder_name: &str,
) -> Result<(), io::Error> {
	info!(
		log,
		"move_update: {:?}, {}", uninstdat_path, update_folder_name
	);

	let root_path = uninstdat_path.parent().ok_or(io::Error::new(
		io::ErrorKind::Other,
		"could not get parent path of uninstdat",
	))?;

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
		.ok_or(io::Error::new(
			io::ErrorKind::Other,
			"could not get current exe name",
		))?
		.to_str()
		.ok_or(io::Error::new(
			io::ErrorKind::Other,
			"could not get current exe name as string",
		))?;

	// delete all current files
	for entry in fs::read_dir(&root_path)? {
		let entry = entry?;
		let entry_name = entry.file_name();
		let entry_name = entry_name.to_str().ok_or(io::Error::new(
			io::ErrorKind::Other,
			"could not get entry name",
		))?;

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

		info!(log, "delete: {:?}", entry_name);

		// attempt to delete
		retry(|| {
			let entry_file_type = entry.file_type()?;
			let entry_path = entry.path();

			info!(log, "attempt to delete: {:?}", entry_name);

			if entry_file_type.is_file() {
				fs::remove_file(&entry_path)?;
			} else {
				fs::remove_dir_all(&entry_path)?;
			}

			if !entry_path.exists() {
				Ok(())
			} else {
				warn!(log, "path still exists: {:?}", entry_name);
				Err(io::Error::new(io::ErrorKind::Other, "path still exists"))
			}
		})?;

		info!(log, "delete OK: {:?}", entry_name);
	}

	// move update to current
	for entry in fs::read_dir(&update_path)? {
		let entry = entry?;
		let entry_name = entry.file_name();
		let entry_name = entry_name.to_str().ok_or(io::Error::new(
			io::ErrorKind::Other,
			"could not get entry name",
		))?;

		let mut target = PathBuf::from(root_path);
		target.push(entry_name);

		info!(log, "rename: {:?}", entry_name);
		retry(|| {
			info!(log, "attempt to rename: {:?}", entry_name);
			fs::rename(entry.path(), &target)
		})?;
		info!(log, "rename OK: {:?}", entry_name);
	}

	fs::remove_dir_all(update_path)
}

fn do_update(
	log: &slog::Logger,
	uninstdat_path: PathBuf,
	update_folder_name: String,
) -> Result<(), io::Error> {
	info!(
		log,
		"do_update: {:?}, {}", uninstdat_path, update_folder_name
	);

	let (header, recs) = read_file(&uninstdat_path)?;

	info!(log, "header: {:?}", header);
	info!(log, "num_recs: {:?}", recs.len());

	let root_path = uninstdat_path.parent().ok_or(io::Error::new(
		io::ErrorKind::Other,
		"could not get parent path of uninstdat",
	))?;

	let mut update_path = PathBuf::from(root_path);
	update_path.push(&update_folder_name);

	move_update(&log, &uninstdat_path, &update_folder_name)?;

	let recs: Vec<FileRec> = recs
		.iter()
		.map(|rec| match rec.typ {
			model::UninstallRecTyp::DeleteDirOrFiles | model::UninstallRecTyp::DeleteFile => {
				rec.rebase(&update_path)
			}
			_ => rec.clone(),
		})
		.collect();

	info!(log, "writing log to {:?}", uninstdat_path);
	write_file(&uninstdat_path, &header, recs)?;

	info!(log, "do_update: done!");

	Ok(())
}

fn update(
	log: &slog::Logger,
	uninstdat_path: PathBuf,
	update_folder_name: String,
	silent: bool,
) -> Result<(), io::Error> {
	// wait a bit before starting
	thread::sleep(time::Duration::from_secs(1));

	if silent {
		do_update(log, uninstdat_path, update_folder_name)
	} else {
		let (tx, rx) = mpsc::channel();

		thread::spawn(move || {
			let window = gui::create_progress_window();
			tx.send(window).unwrap();

			gui::event_loop();
		});

		let window = rx.recv()
			.map_err(|_| io::Error::new(io::ErrorKind::Other, "Could not receive GUI window handle"))?;

		do_update(&log, uninstdat_path, update_folder_name)?;
		window.exit();

		Ok(())
	}
}

fn _main(log: &slog::Logger) -> Result<(), io::Error> {
	info!(log, "Starting");

	let args: Vec<String> = env::args().filter(|a| !a.starts_with("--")).collect();

	if args.len() < 4 {
		return Err(io::Error::new(
			io::ErrorKind::Other,
			"Usage: inno_updater.exe update_folder_name app_path silent",
		));
	}

	let update_folder_name = args[1].clone();
	let uninstdat_path = PathBuf::from(&args[2]);
	let silent = args[3].clone();

	if !uninstdat_path.is_absolute() {
		return Err(io::Error::new(
			io::ErrorKind::Other,
			"Uninstdat path needs to be absolute",
		));
	}

	if silent != "true" && silent != "false" {
		return Err(io::Error::new(
			io::ErrorKind::Other,
			"Silent needs to be true or false",
		));
	}

	update(log, uninstdat_path, update_folder_name, silent == "true")
}

fn __main() -> i32 {
	let mut log_path = env::temp_dir();
	log_path.push(format!("vscode-inno-updater.log"));

	let file = fs::OpenOptions::new()
		.create(true)
		.write(true)
		.truncate(true)
		.open(&log_path)
		.unwrap();

	let decorator = slog_term::PlainDecorator::new(file);
	let drain = slog_term::FullFormat::new(decorator).build().fuse();
	let drain = slog_async::Async::new(drain).build().fuse();
	let log = slog::Logger::root(drain, o!());

	match _main(&log) {
		Ok(_) => 0,
		Err(err) => {
			error!(log, "{}", err);
			let msg = format!("Failed to install VS Code update. Please download and reinstall VS Code.\n\nPlease attach the following log file to a new issue on GitHub:\n\n{}", log_path.to_str().unwrap());
			gui::message_box(&msg, "VS Code");

			1
		}
	}
}

fn main() {
	std::process::exit(__main());
}

// fn main() {
// 	let window = gui::create_progress_window();

// 	thread::spawn(move || {
// 		thread::sleep(time::Duration::from_secs(5));
// 		window.exit();
// 	});

// 	gui::event_loop();
// }
