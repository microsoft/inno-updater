/*-----------------------------------------------------------------------------------------
 *  Copyright (c) Microsoft Corporation. All rights reserved.
 *  Licensed under the MIT License. See LICENSE in the project root for license information.
 *----------------------------------------------------------------------------------------*/

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

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
mod process;
mod util;

use std::{env, error, fmt, fs, io, thread};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::io::prelude::*;
use std::vec::Vec;
use slog::Drain;
use model::{FileRec, Header};

fn read_file(path: &Path) -> Result<(Header, Vec<FileRec>), Box<error::Error>> {
	let input_file = fs::File::open(path)?;
	let mut input = io::BufReader::new(input_file);

	let header = Header::from_reader(&mut input)?;
	let mut reader = blockio::BlockRead::new(&mut input);
	let mut recs = Vec::with_capacity(header.num_recs);

	for _ in 0..header.num_recs {
		recs.push(FileRec::from_reader(&mut reader)?);
	}

	Ok((header, recs))
}

fn write_file(path: &Path, header: &Header, recs: Vec<FileRec>) -> Result<(), Box<error::Error>> {
	let mut output_file = fs::File::create(path)?;

	// skip header
	output_file.seek(io::SeekFrom::Start(448))?;

	{
		let mut output = io::BufWriter::new(&output_file);
		let mut writer = blockio::BlockWrite::new(&mut output);

		for rec in recs {
			rec.to_writer(&mut writer)?;
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
	header.to_writer(&mut output)?;

	output.flush()?;

	Ok(())
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
		if entry_name == "tools" {
			continue;
		}

		// attempt to delete
		util::retry(|attempt| {
			let entry_file_type = entry.file_type()?;
			let entry_path = entry.path();

			info!(log, "Delete: {:?} (attempt {})", entry_name, attempt);

			if entry_file_type.is_file() {
				fs::remove_file(&entry_path)?;
			} else {
				fs::remove_dir_all(&entry_path)?;
			}

			if !entry_path.exists() {
				Ok(())
			} else {
				warn!(log, "Path still exists: {:?}", entry_name);
				Err(io::Error::new(io::ErrorKind::Other, "path still exists"))
			}
		})?;
	}

	// move update to current
	for entry in fs::read_dir(&update_path)? {
		let entry = entry?;
		let entry_name = entry.file_name();
		let entry_name = entry_name.to_str().ok_or(io::Error::new(
			io::ErrorKind::Other,
			"Could not get entry name",
		))?;

		let mut target = PathBuf::from(root_path);
		target.push(entry_name);

		util::retry(|attempt| {
			info!(log, "Rename: {:?} (attempt {})", entry_name, attempt);
			fs::rename(entry.path(), &target)
		})?;
	}

	info!(log, "Delete: {:?}", update_path);
	fs::remove_dir_all(update_path)
}

fn do_update(
	log: &slog::Logger,
	code_path: &PathBuf,
	update_folder_name: &str,
) -> Result<(), Box<error::Error>> {
	info!(log, "do_update: {:?}, {}", code_path, update_folder_name);

	let root_path = code_path.parent().ok_or(io::Error::new(
		io::ErrorKind::Other,
		"could not get parent path of uninstdat",
	))?;

	let mut uninstdat_path = PathBuf::from(root_path);
	uninstdat_path.push("unins000.dat");

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

	let recs: Result<Vec<FileRec>, _> = recs
		.iter()
		.map(|rec| match rec.typ {
			model::UninstallRecTyp::DeleteDirOrFiles | model::UninstallRecTyp::DeleteFile => {
				rec.rebase(&update_path)
			}
			_ => Ok(rec.clone()),
		})
		.collect();

	info!(log, "Updating uninstall file {:?}", uninstdat_path);
	write_file(&uninstdat_path, &header, recs?)?;

	Ok(())
}

fn update(
	log: &slog::Logger,
	code_path: &PathBuf,
	update_folder_name: &str,
	silent: bool,
) -> Result<(), Box<error::Error>> {
	process::wait_or_kill(log, code_path)?;

	info!(log, "Starting update, silent = {}", silent);

	let (tx, rx) = mpsc::channel();

	thread::spawn(move || {
		let window = gui::create_progress_window(silent);
		tx.send(window).unwrap();

		gui::event_loop();
	});

	let window = rx.recv()
		.map_err(|_| io::Error::new(io::ErrorKind::Other, "Could not receive GUI window handle"))?;

	do_update(&log, code_path, update_folder_name)?;
	window.exit();

	Ok(())
}

#[derive(Debug, Clone)]
struct ArgumentError(String);

impl fmt::Display for ArgumentError {
	fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
		write!(f, "Bad arguments: {}", self.0)
	}
}

impl error::Error for ArgumentError {
	fn description(&self) -> &str {
		"ArgumentError"
	}

	fn cause(&self) -> Option<&error::Error> {
		None
	}
}

fn _main(log: &slog::Logger, args: &Vec<String>) -> Result<(), Box<error::Error>> {
	info!(log, "Starting: {}, {}", args[1], args[2]);

	let code_path = PathBuf::from(&args[1]);

	if !code_path.is_absolute() {
		return Err(
			ArgumentError(format!(
				"Code path needs to be absolute. Instead got: {}",
				args[1]
			)).into(),
		);
	}

	if !code_path.exists() {
		return Err(ArgumentError(format!("Code path doesn't seem to exist: {}", args[1])).into());
	}

	let silent = args[2].clone();

	if silent != "true" && silent != "false" {
		return Err(
			ArgumentError(format!(
				"Silent needs to be true or false. Instead got: {}",
				silent
			)).into(),
		);
	}

	update(log, &code_path, "_", silent == "true")
}

fn __main(args: &Vec<String>) -> i32 {
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

	match _main(&log, args) {
		Ok(_) => {
			info!(log, "Update was successful!");
			0
		}
		Err(err) => {
			error!(log, "{}", err);

			let msg = format!(
				"Failed to install VS Code update. Please download and reinstall VS Code.

Please attach the following log file to a new issue on GitHub:

{}",
				log_path.to_str().unwrap()
			);

			gui::message_box(&msg, "VS Code");

			1
		}
	}
}

fn parse(path: &Path) -> Result<(), Box<error::Error>> {
	let (header, recs) = read_file(path)?;

	println!("{:?}", header);

	use std::collections::HashMap;
	let mut map: HashMap<u16, u32> = HashMap::new();

	for rec in recs {
		let count = map.entry(rec.typ as u16).or_insert(0);
		*count += 1;
	}

	for (k, c) in &map {
		println!("records 0x{:x} {}", k, c);
	}

	Ok(())
}

fn main() {
	let args: Vec<String> = env::args().collect();

	if args.len() == 3 && args[1] == "--parse" {
		let path = PathBuf::from(&args[2]);
		parse(&path).unwrap_or_else(|err| {
			eprintln!("{}", err);
			std::process::exit(1);
		});
	} else if args.len() == 2 && args[1] == "--gui" {
		let window = gui::create_progress_window(false);

		thread::spawn(move || {
			thread::sleep(std::time::Duration::from_secs(5));
			window.exit();
		});

		gui::event_loop();
	} else {
		let args: Vec<String> = args.into_iter().filter(|a| !a.starts_with("--")).collect();

		if args.len() < 3 {
			eprintln!("Bad usage");
			std::process::exit(1);
		} else {
			std::process::exit(__main(&args));
		}
	}
}
