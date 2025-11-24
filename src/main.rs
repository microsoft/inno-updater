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
extern crate windows_sys;
#[cfg(test)]
extern crate tempfile;

mod blockio;
mod gui;
mod handle;
mod model;
mod process;
mod resources;
mod strings;
mod util;

use handle::FileHandle;
use model::{FileRec, Header};
use slog::Drain;
use std::collections::{HashSet, LinkedList};
use std::io::prelude::*;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::SystemTime;
use std::vec::Vec;
use std::{env, error, fmt, fs, io, thread};

const VERSION: &str = env!("CARGO_PKG_VERSION");

fn read_file(path: &Path) -> Result<(Header, Vec<FileRec>), Box<dyn error::Error>> {
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

fn write_file(
	path: &Path,
	header: &Header,
	recs: Vec<FileRec>,
) -> Result<(), Box<dyn error::Error>> {
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
	let end_offset = output_file.stream_position()?;
	header.end_offset = end_offset as u32;

	// go back to beginning
	output_file.seek(io::SeekFrom::Start(0))?;

	let mut output = io::BufWriter::new(&output_file);
	header.to_writer(&mut output)?;

	output.flush()?;

	Ok(())
}

fn delete_existing_version(
	log: &slog::Logger,
	root_path: &Path,
	update_folder_name: &str,
) -> Result<(), Box<dyn error::Error>> {
	let mut directories: LinkedList<PathBuf> = LinkedList::new();
	let mut top_directories: LinkedList<PathBuf> = LinkedList::new();
	let mut file_handles: LinkedList<FileHandle> = LinkedList::new();

	let root = PathBuf::from(root_path);
	directories.push_back(root);

	while let Some(dir) = directories.pop_front() {
		info!(log, "Reading directory: {:?}", dir);

		for entry in fs::read_dir(&dir)? {
			let entry = entry?;
			let entry_name = entry.file_name();
			let entry_name = entry_name
				.to_str()
				.ok_or_else(|| io::Error::new(io::ErrorKind::Other, "Could not get entry name"))?;

			if dir == root_path {
				// don't delete the update folder
				if entry_name == update_folder_name {
					continue;
				}

				// don't delete ourselves
				if entry_name == "tools" {
					continue;
				}

				// don't delete any of the unins* files
				if entry_name.starts_with("unins") {
					continue;
				}

				// don't delete the sparse package folder
				if entry_name == "appx" {
					continue;
				}

				// don't delete the bootstrap folder
				if entry_name == "bootstrap" {
					continue;
				}
			}

			let entry_file_type = entry.file_type()?;
			let entry_path = entry.path();

			if entry_file_type.is_dir() {
				if dir == root_path {
					top_directories.push_back(entry_path.to_owned());
				}

				directories.push_back(entry_path);
			} else if entry_file_type.is_file() {
				// attempt to get exclusive file handle
				let msg = format!("Opening file handle: {:?}", entry_path);
				let file_handle = util::retry(
					&msg,
					|attempt| -> Result<FileHandle, Box<dyn error::Error>> {
						info!(
							log,
							"Get file handle: {:?} (attempt {})", entry_path, attempt
						);

						FileHandle::new(&entry_path)
					},
					Some(16),
				)?;

				file_handles.push_back(file_handle);
			}
		}
	}

	info!(log, "Collected all directories and file handles");

	for file_handle in &file_handles {
		util::retry(
			"marking a file for deletion",
			|_| -> Result<(), Box<dyn error::Error>> { file_handle.mark_for_deletion() },
			None,
		)?;
	}

	info!(log, "All file handles marked for deletion");

	for file_handle in &file_handles {
		util::retry(
			"closing a file handle",
			|_| -> Result<(), Box<dyn error::Error>> { file_handle.close() },
			None,
		)?;
	}

	info!(log, "All files deleted");

	for dir in top_directories {
		let msg = format!("Deleting a directory: {:?}", dir);
		util::retry(
			&msg,
			|attempt| -> Result<(), Box<dyn error::Error>> {
				if !dir.exists() {
					return Ok(());
				}

				info!(
					log,
					"Delete directory recursively: {:?} (attempt {})", dir, attempt
				);

				fs::remove_dir_all(&dir)?;
				Ok(())
			},
			None,
		)?;
	}

	Ok(())
}

fn move_update(
	log: &slog::Logger,
	uninstdat_path: &Path,
	update_folder_name: &str,
) -> Result<(), Box<dyn error::Error>> {
	info!(
		log,
		"move_update: {:?}, {}", uninstdat_path, update_folder_name
	);

	let root_path = uninstdat_path.parent().ok_or_else(|| {
		io::Error::new(
			io::ErrorKind::Other,
			"Could not get parent path of uninstdat",
		)
	})?;

	let mut update_path = PathBuf::from(root_path);
	update_path.push(update_folder_name);

	let stat = fs::metadata(&update_path)?;

	if !stat.is_dir() {
		return Err(
			io::Error::new(io::ErrorKind::Other, "Update folder is not a directory").into(),
		);
	}

	// safely delete all current files
	delete_existing_version(log, root_path, update_folder_name)?;

	// move update to current
	for entry in fs::read_dir(&update_path)? {
		let entry = entry?;
		let entry_name = entry.file_name();
		let entry_name = entry_name
			.to_str()
			.ok_or_else(|| io::Error::new(io::ErrorKind::Other, "Could not get entry name"))?;

		let mut target = PathBuf::from(root_path);
		target.push(entry_name);

		let msg = format!("Renaming: {:?}", entry_name);
		util::retry(
			&msg,
			|attempt| {
				info!(log, "Rename: {:?} (attempt {})", entry_name, attempt);
				fs::rename(entry.path(), &target)?;
				Ok(())
			},
			None,
		)?;
	}

	info!(log, "Delete: {:?}", update_path);
	fs::remove_dir_all(update_path)?;

	Ok(())
}

fn patch_uninstdat(
	log: &slog::Logger,
	uninstdat_path: &PathBuf,
	update_path: &PathBuf,
) -> Result<(), Box<dyn error::Error>> {
	let (header, recs) = read_file(uninstdat_path)?;

	info!(log, "header: {:?}", header);
	info!(log, "num_recs: {:?}", recs.len());

	let recs: Vec<FileRec> = recs
		.iter()
		.map(|rec| match rec.typ {
			model::UninstallRecTyp::DeleteDirOrFiles | model::UninstallRecTyp::DeleteFile => {
				rec.rebase(&update_path)
			}
			_ => Ok(rec.clone()),
		})
		.collect::<Result<Vec<_>, _>>()?;

	// Remove duplicate records of type DeleteDirOrFiles and DeleteFile that only have one path
	let before = recs.len();
	let mut set: HashSet<String> = HashSet::new();
	let recs = recs
		.into_iter()
		.filter(|rec| {
			if rec.typ != model::UninstallRecTyp::DeleteDirOrFiles
				&& rec.typ != model::UninstallRecTyp::DeleteFile
			{
				return true;
			}

			match rec.get_paths() {
				Ok(paths) => {
					if paths.len() != 1 {
						return true;
					}

					let path = &paths[0];
					if set.contains(path) {
						return false;
					}

					set.insert(path.clone());
					true
				}
				Err(_) => false, // Skip records with errors in paths
			}
		})
		.collect::<Vec<FileRec>>();

	let header = header.clone_with_num_recs(recs.len());
	info!(log, "Removed {} duplicate records", before - recs.len());

	info!(log, "Updating uninstall file {:?}", uninstdat_path);
	write_file(uninstdat_path, &header, recs)?;

	Ok(())
}

fn do_update(
	log: &slog::Logger,
	code_path: &PathBuf,
	update_folder_name: &str,
) -> Result<(), Box<dyn error::Error>> {
	info!(log, "do_update: {:?}, {}", code_path, update_folder_name);

	let root_path = code_path.parent().ok_or_else(|| {
		io::Error::new(
			io::ErrorKind::Other,
			"Could not get parent path of uninstdat",
		)
	})?;

	let mut uninstdat_path = PathBuf::from(root_path);
	uninstdat_path.push("unins000.dat");

	move_update(log, &uninstdat_path, update_folder_name)?;

	let root_path = uninstdat_path.parent().ok_or_else(|| {
		io::Error::new(
			io::ErrorKind::Other,
			"Could not get parent path of uninstdat",
		)
	})?;

	let mut update_path = PathBuf::from(root_path);
	update_path.push(update_folder_name);

	// if, for any reason, the uninstdat file is corrupt, let's continue silently
	// https://github.com/Microsoft/vscode/issues/45607
	patch_uninstdat(log, &uninstdat_path, &update_path).unwrap_or_else(|err| {
		warn!(log, "Failed to patch uninst.dat file");
		warn!(log, "{}", err);
	});

	Ok(())
}

fn update(
	log: &slog::Logger,
	code_path: &PathBuf,
	update_folder_name: &str,
	silent: bool,
	label: String,
	commit: Option<String>,
) -> Result<(), Box<dyn error::Error>> {
	info!(log, "Inno Updater v{}", VERSION);
	info!(log, "Starting update, silent = {}", silent);

	let (tx, rx) = mpsc::channel();

	thread::spawn(move || {
		gui::run_progress_window(silent, tx, label);
	});

	let window = rx
		.recv()
		.map_err(|_| io::Error::new(io::ErrorKind::Other, "Could not receive GUI window handle"))?;

	// 2) Get the basename and dirname of code_path
	let dir_path = code_path.parent().ok_or_else(|| {
		io::Error::new(io::ErrorKind::Other, "Could not get parent directory of code_path")
	})?;

	let basename = code_path.file_name().ok_or_else(|| {
		io::Error::new(io::ErrorKind::Other, "Could not get basename of code_path")
	})?;

	let basename_str = basename.to_string_lossy();

	// 3) Create variables for old_{basename} and new_{basename}
	let old_exe_filename = format!("old_{}", basename_str);
	let new_exe_filename = format!("new_{}", basename_str);

	let old_exe_path = dir_path.join(&old_exe_filename);
	let new_exe_path = dir_path.join(&new_exe_filename);

	info!(log, "Starting rename process: code_path={:?}, old_exe_path={:?}, new_exe_path={:?}", 
		code_path, old_exe_path, new_exe_path);

	// 4) Check for the presence of new_exe_filename and proceed with renaming
	if new_exe_path.exists() {
		info!(log, "Found new executable: {:?}", new_exe_path);

		// 5) Handle the bin folder files with 3-way rename
		let bin_dir = dir_path.join("bin");
		if bin_dir.exists() {
			info!(log, "Processing bin directory: {:?}", bin_dir);

			// Collect all files in the bin directory for processing
			let mut bin_files = Vec::new();
			if let Ok(entries) = fs::read_dir(&bin_dir) {
				for entry in entries {
					if let Ok(entry) = entry {
						let file_name = entry.file_name();
						let file_name_str = file_name.to_string_lossy();

						// Skip files that already have old_ or new_ prefix
						if !file_name_str.starts_with("old_") && !file_name_str.starts_with("new_") {
							bin_files.push(file_name.to_string_lossy().to_string());
						}
					}
				}
			}

			// Track files that were successfully renamed for potential rollback
			let mut renamed_files = Vec::new();

			// Process each file in the bin directory
			for file_name in bin_files {
				let current_file = bin_dir.join(&file_name);
				let old_file = bin_dir.join(format!("old_{}", file_name));
				let new_file = bin_dir.join(format!("new_{}", file_name));

				// Perform three-way rename for bin file
				if new_file.exists() {
					info!(log, "Found new bin file: {:?}", new_file);
					window.update_status("Renaming files under bin folder...");
					match perform_three_way_rename(log, &current_file, &old_file, &new_file) {
						Ok(_) => {
							// Track this file was successfully renamed
							renamed_files.push(file_name);
						},
						Err(err) => {
							error!(log, "Bin file update failed for {:?}: {}", file_name, err);
							// Continue to next file, don't rollback everything yet
							continue;
						}
					}
				}
			}

			info!(log, "Bin directory processing complete. Successfully renamed {} files", renamed_files.len());
		} else {
			info!(log, "Bin directory does not exist, skipping bin file processing");
		}

		// Perform three-way rename for the main executable
		window.update_status("Renaming main executable...");
		if let Err(err) = perform_three_way_rename(log, code_path, &old_exe_path, &new_exe_path) {
			error!(log, "Executable update failed: {}", err);
			window.exit();
			return Err(err);
		}

		// Also perform three-way rename for the VisualElementsManifest.xml file
		let basename_without_ext = basename_str.strip_suffix(".exe").unwrap_or(&basename_str);
		let manifest_filename = format!("{}.VisualElementsManifest.xml", basename_without_ext);
		let manifest_path = dir_path.join(&manifest_filename);
		let old_manifest_filename = format!("old_{}", manifest_filename);
		let new_manifest_filename = format!("new_{}", manifest_filename);
		let old_manifest_path = dir_path.join(&old_manifest_filename);
		let new_manifest_path = dir_path.join(&new_manifest_filename);

		if new_manifest_path.exists() {
			window.update_status("Renaming manifest file...");
			info!(log, "Found new manifest file: {:?}", new_manifest_path);
			if let Err(err) = perform_three_way_rename(log, &manifest_path, &old_manifest_path, &new_manifest_path) {
				error!(log, "Manifest file update failed: {}", err);
			} else {
				info!(log, "Successfully updated manifest file");
			}
		} else {
			info!(log, "No new manifest file found: {:?}", new_manifest_path);
		}

		window.update_status("Attempting to stop current running application...");
		process::wait_or_kill(log, code_path)?;

		// If a commit argument was provided, attempt to remove files not associated with that commit
		if let Some(ref commit_str) = commit {
			window.update_status("Cleaning up old files...");
			info!(log, "Commit specified: {} - attempting to remove files", commit_str);
			if let Err(err) = remove_files(log, code_path, commit_str) {
				warn!(log, "Failed to remove files for commit {}: {}", commit_str, err);
			} else {
				info!(log, "Removed files for commit {}", commit_str);
			}
		} else {
			// Clean up DLL files if transitioning from old layout to new layout
			// as part of rename so that DLL doesn't get injected into the new
			// application launch.
			window.update_status("Cleaning up DLL files...");
			if let Err(err) = cleanup_dll_files(log, code_path) {
				warn!(log, "Failed to cleanup DLL files: {}", err);
			}
		}

		window.update_status("Update completed successfully!");
		info!(log, "Update completed successfully");
	} else {
		info!(log, "New executable not found: {:?}, using traditional update method", new_exe_path);
		// Fall back to the original update method if no new executable is found
		do_update(log, code_path, update_folder_name)?;
	}

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

	fn cause(&self) -> Option<&dyn error::Error> {
		None
	}
}

fn _main(log: &slog::Logger, args: &[String]) -> Result<(), Box<dyn error::Error>> {
	info!(log, "Starting: {}, {}, {}", args[1], args[2], args[3]);

	let code_path = PathBuf::from(&args[1]);

	if !code_path.is_absolute() {
		return Err(ArgumentError(format!(
			"Code path needs to be absolute. Instead got: {}",
			args[1]
		))
		.into());
	}

	if !code_path.exists() {
		return Err(ArgumentError(format!("Code path doesn't seem to exist: {}", args[1])).into());
	}

	let silent = args[2].clone();

	if silent != "true" && silent != "false" {
		return Err(ArgumentError(format!(
			"Silent needs to be true or false. Instead got: {}",
			silent
		))
		.into());
	}

	let label = args[3].clone();

	// optional commit arg in args[4]
	let commit = if args.len() > 4 {
		Some(args[4].clone())
	} else {
		None
	};

	update(log, &code_path, "_", silent == "true", label, commit)
}

fn handle_error(log_path: &str) {
	let msg = format!(
		"Failed to install Visual Studio Code update.\n\n\
		Updates may fail due to anti-virus software and/or runaway processes. Please try restarting your machine before attempting to update again.\n\n\
		Please read the log file for more information:\n\n\
		{log_path}"
	);

	gui::message_box(&msg, "Visual Studio Code", gui::MessageBoxType::Error);
}

fn parse(path: &Path) -> Result<(), Box<dyn error::Error>> {
	let (header, recs) = read_file(path)?;

	println!("{:?}", header);

	use std::collections::HashMap;
	let mut map: HashMap<u16, u32> = HashMap::new();

	println!("Paths");
	for rec in recs {
		let count = map.entry(rec.typ as u16).or_insert(0);
		*count += 1;

		match rec.typ {
			model::UninstallRecTyp::DeleteDirOrFiles | model::UninstallRecTyp::DeleteFile => {
				let paths = rec.get_paths().unwrap();
				for path in paths {
					println!("  {}", path);
				}
			}
			_ => {}
		}
	}

	println!("Summary");
	let mut records: Vec<_> = map.into_iter().collect();
	records.sort_by(|a, b| a.0.cmp(&b.0));
	for (k, c) in &records {
		println!("  {} records of type 0x{:x}", c, k);
	}

	Ok(())
}

fn perform_three_way_rename(
	log: &slog::Logger,
	current_path: &Path,
	old_path: &Path,
	new_path: &Path,
) -> Result<(), Box<dyn error::Error>> {
	// Step 1: If new file exists and current file exists, rename current to old
	if new_path.exists() && current_path.exists() {
		info!(log, "Renaming current to old: {:?} -> {:?}", current_path, old_path);
		if let Err(err) = fs::rename(current_path, old_path) {
			error!(log, "Failed to rename current to old: {}", err);
			return Err(Box::new(io::Error::new(
				io::ErrorKind::Other,
				format!("Failed to rename current to old: {}", err),
			)));
		}
	} else if !new_path.exists() {
		// No new file to rename, so nothing to do
		return Ok(());
	}

	// Step 2: Rename new to current
	info!(log, "Renaming new to current: {:?} -> {:?}", new_path, current_path);
	if let Err(err) = fs::rename(new_path, current_path) {
		error!(log, "Failed to rename new to current, attempting to restore old: {}", err);

		// Restore old file if the operation fails and old file exists
		if old_path.exists() {
			info!(log, "Restoring old file: {:?} -> {:?}", old_path, current_path);
			if let Err(restore_err) = fs::rename(old_path, current_path) {
				error!(log, "Failed to restore old file: {}", restore_err);
			}
		}

		return Err(Box::new(io::Error::new(
			io::ErrorKind::Other,
			format!("Failed to rename new to current: {}", err),
		)));
	}

	Ok(())
}

fn cleanup_dll_files(
	log: &slog::Logger,
	code_path: &Path,
) -> Result<(), Box<dyn error::Error>> {
	info!(log, "cleanup_dll_files: {:?}", code_path);

	let dir_path = code_path.parent().ok_or_else(|| {
		io::Error::new(
			io::ErrorKind::Other,
			"Could not get parent directory of code_path",
		)
	})?;

	// Check for ffmpeg.dll
	let ffmpeg_path = dir_path.join("ffmpeg.dll");
	if !ffmpeg_path.exists() {
		info!(log, "ffmpeg.dll not found, skipping DLL cleanup");
		return Ok(());
	}

	info!(log, "ffmpeg.dll found at {:?}, removing all DLL files from directory", ffmpeg_path);

	let mut file_handles_to_remove: LinkedList<FileHandle> = LinkedList::new();

	// Scan directory for DLL files
	for entry in fs::read_dir(dir_path)? {
		let entry = entry?;
		let entry_path = entry.path();
		let entry_file_type = entry.file_type()?;

		if entry_file_type.is_file() {
			if let Some(extension) = entry_path.extension() {
				if extension.eq_ignore_ascii_case("dll") {
					info!(log, "Found DLL file to remove: {:?}", entry_path);

					let msg = format!("Opening file handle: {:?}", entry_path);
					let file_handle = util::retry(
						&msg,
						|attempt| -> Result<FileHandle, Box<dyn error::Error>> {
							info!(
								log,
								"Get file handle: {:?} (attempt {})", entry_path, attempt
							);

							FileHandle::new(&entry_path)
						},
						Some(16),
					)?;

					file_handles_to_remove.push_back(file_handle);
				}
			}
		}
	}

	if file_handles_to_remove.is_empty() {
		info!(log, "No DLL files found to remove");
		return Ok(());
	}

	info!(log, "Collected {} DLL file handles for removal", file_handles_to_remove.len());

	for file_handle in &file_handles_to_remove {
		util::retry(
			"marking a DLL file for deletion",
			|_| -> Result<(), Box<dyn error::Error>> { file_handle.mark_for_deletion() },
			None,
		)?;
	}

	info!(log, "All DLL file handles marked for deletion");

	for file_handle in &file_handles_to_remove {
		util::retry(
			"closing a DLL file handle",
			|_| -> Result<(), Box<dyn error::Error>> { file_handle.close() },
			None,
		)?;
	}

	info!(log, "All DLL files deleted");
	Ok(())
}

fn remove_files(
	log: &slog::Logger,
	code_path: &Path,
	commit_to_preserve: &str,
) -> Result<(), Box<dyn error::Error>> {
	info!(log, "remove_files: {:?}, commit: {}", code_path, commit_to_preserve);

	let base_dir = code_path.parent().ok_or_else(|| {
		io::Error::new(
			io::ErrorKind::Other,
			"Could not get parent directory of code_path",
		)
	})?;

	let code_basename = code_path.file_name().ok_or_else(|| {
		io::Error::new(
			io::ErrorKind::Other,
			"Could not get basename of code_path",
		)
	})?;

	let code_basename_str = code_basename.to_string_lossy();
	let basename_without_ext = code_basename_str.strip_suffix(".exe").unwrap_or(&code_basename_str);
	let manifest_filename = format!("{}.VisualElementsManifest.xml", basename_without_ext);

	let mut directories_to_remove: LinkedList<PathBuf> = LinkedList::new();
	let mut file_handles_to_remove: LinkedList<FileHandle> = LinkedList::new();

	info!(log, "Reading top-level directory: {:?}", base_dir);

	// Only process top-level contents of base_dir
	for entry in fs::read_dir(base_dir)? {
		let entry = entry?;
		let entry_name = entry.file_name();
		let entry_name = entry_name
			.to_str()
			.ok_or_else(|| io::Error::new(io::ErrorKind::Other, "Could not get entry name"))?;

		let entry_file_type = entry.file_type()?;
		let entry_path = entry.path();

		let should_skip = 
			// Skip deleting code_path executable
			if entry_path == code_path {
				info!(log, "Skipping code_path executable: {:?}", entry_path);
				true
			}
			// Skip basename.VisualElementsManifest.xml
			else if entry_name == manifest_filename {
				info!(log, "Skipping VisualElementsManifest.xml: {:?}", entry_path);
				true
			}
			// Skip files starting with "unins"
			else if entry_name.starts_with("unins") {
				info!(log, "Skipping unins file: {:?}", entry_path);
				true
			}
			// Skip commit folder
			else if entry_name == commit_to_preserve && entry_file_type.is_dir() {
				info!(log, "Skipping commit folder: {:?}", entry_path);
				true
			}
			// Skip bootstrap folder
			else if entry_name == "bootstrap" {
				info!(log, "Skipping bootstrap folder: {:?}", entry_path);
				true
			}
			else {
				false
			};

		if should_skip {
			continue;
		}

		if entry_file_type.is_dir() {
			// Special handling for bin directory (Rule 3)
			if entry_name == "bin" {
				info!(log, "Processing bin directory: {:?}", entry_path);
				
				// Process files in bin directory
				for bin_entry in fs::read_dir(&entry_path)? {
					let bin_entry = bin_entry?;
					let bin_entry_name = bin_entry.file_name();
					let bin_entry_name = bin_entry_name
						.to_str()
						.ok_or_else(|| io::Error::new(io::ErrorKind::Other, "Could not get bin entry name"))?;

					let bin_entry_file_type = bin_entry.file_type()?;
					let bin_entry_path = bin_entry.path();

					// In bin folder, only delete files starting with "old_"
					if bin_entry_file_type.is_file() {
						if bin_entry_name.starts_with("old_") {
							info!(log, "Will delete old file in bin: {:?}", bin_entry_path);
							
							let msg = format!("Opening file handle: {:?}", bin_entry_path);
							let file_handle = util::retry(
								&msg,
								|attempt| -> Result<FileHandle, Box<dyn error::Error>> {
									info!(
										log,
										"Get file handle: {:?} (attempt {})", bin_entry_path, attempt
									);

									FileHandle::new(&bin_entry_path)
								},
								Some(16),
							)?;

							file_handles_to_remove.push_back(file_handle);
						} else {
							info!(log, "Skipping non-old file in bin: {:?}", bin_entry_path);
						}
					}
				}

				// Don't add bin directory itself to top_directories for deletion
			} else {
				// Delete other directories entirely
				directories_to_remove.push_back(entry_path.to_owned());
			}
		} else if entry_file_type.is_file() {
			// Delete top-level files (except those already skipped)
			let msg = format!("Opening file handle: {:?}", entry_path);
			let file_handle = util::retry(
				&msg,
				|attempt| -> Result<FileHandle, Box<dyn error::Error>> {
					info!(
						log,
						"Get file handle: {:?} (attempt {})", entry_path, attempt
					);

					FileHandle::new(&entry_path)
				},
				Some(16),
			)?;

			file_handles_to_remove.push_back(file_handle);
		}
	}

	info!(log, "Collected all directories and file handles for removal");

	for file_handle in &file_handles_to_remove {
		util::retry(
			"marking a file for deletion",
			|_| -> Result<(), Box<dyn error::Error>> { file_handle.mark_for_deletion() },
			None,
		)?;
	}

	info!(log, "All file handles marked for deletion");

	for file_handle in &file_handles_to_remove {
		util::retry(
			"closing a file handle",
			|_| -> Result<(), Box<dyn error::Error>> { file_handle.close() },
			None,
		)?;
	}

	info!(log, "All files deleted");

	for dir in directories_to_remove {
		let msg = format!("Deleting a directory: {:?}", dir);
		util::retry(
			&msg,
			|attempt| -> Result<(), Box<dyn error::Error>> {
				if !dir.exists() {
					return Ok(());
				}

				info!(
					log,
					"Delete directory recursively: {:?} (attempt {})", dir, attempt
				);

				fs::remove_dir_all(&dir)?;
				Ok(())
			},
			None,
		)?;
	}

	info!(log, "File removal operation completed");
	Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;
    use slog::{Logger, o};
    use slog_term::{TermDecorator, FullFormat};
    use slog_async::Async;

    // Helper function to set up a test logger
    fn setup_test_logger() -> Logger {
        let decorator = TermDecorator::new().build();
        let drain = FullFormat::new(decorator).build().fuse();
        let drain = Async::new(drain).build().fuse();
        Logger::root(drain, o!())
    }

    #[test]
    fn test_perform_three_way_rename_basic() {
        // Create a temporary directory for our test
        let temp_dir = tempdir().unwrap();
        let log = setup_test_logger();

        // Set up our test paths
        let current_path = temp_dir.path().join("current.txt");
        let old_path = temp_dir.path().join("old_current.txt");
        let new_path = temp_dir.path().join("new_current.txt");

        // Create test files
        fs::write(&current_path, "current content").unwrap();
        fs::write(&new_path, "new content").unwrap();

        // Perform the rename operation
        let result = perform_three_way_rename(&log, &current_path, &old_path, &new_path);

        // Verify results
        assert!(result.is_ok(), "Rename operation should succeed");
        assert!(current_path.exists(), "Current file should exist");
        assert!(!new_path.exists(), "New file should be renamed");
        assert!(old_path.exists(), "Old file should exist");

        // Verify content
        let current_content = fs::read_to_string(&current_path).unwrap();
        let old_content = fs::read_to_string(&old_path).unwrap();
        assert_eq!(current_content, "new content", "Current file should contain new content");
        assert_eq!(old_content, "current content", "Old file should contain original content");
    }

    #[test]
    fn test_perform_three_way_rename_no_current_file() {
        let temp_dir = tempdir().unwrap();
        let log = setup_test_logger();

        let current_path = temp_dir.path().join("current.txt");
        let old_path = temp_dir.path().join("old_current.txt");
        let new_path = temp_dir.path().join("new_current.txt");

        // Only create the new file (no current file)
        fs::write(&new_path, "new content").unwrap();

        // Perform the rename operation
        let result = perform_three_way_rename(&log, &current_path, &old_path, &new_path);

        // Verify results
        assert!(result.is_ok(), "Rename operation should succeed even without current file");
        assert!(current_path.exists(), "Current file should exist after rename");
        assert!(!new_path.exists(), "New file should be renamed");
        assert!(!old_path.exists(), "Old file should not exist as there was no current file");

        // Verify content
        let current_content = fs::read_to_string(&current_path).unwrap();
        assert_eq!(current_content, "new content", "Current file should contain new content");
    }

    #[test]
    fn test_perform_three_way_rename_no_new_file() {
        let temp_dir = tempdir().unwrap();
        let log = setup_test_logger();

        let current_path = temp_dir.path().join("current.txt");
        let old_path = temp_dir.path().join("old_current.txt");
        let new_path = temp_dir.path().join("new_current.txt");

        // Only create the current file (no new file)
        fs::write(&current_path, "current content").unwrap();

        // Perform the rename operation
        let result = perform_three_way_rename(&log, &current_path, &old_path, &new_path);

        // Verify results
        assert!(result.is_ok(), "Rename operation should return Ok when there's no new file");
        assert!(current_path.exists(), "Current file should still exist");
        assert!(!old_path.exists(), "Old file should not exist as rename wasn't needed");

        // Verify content is unchanged
        let current_content = fs::read_to_string(&current_path).unwrap();
        assert_eq!(current_content, "current content", "Current file should be unchanged");
    }

    #[test]
    fn test_remove_files_basic() {
        let temp_dir = tempdir().unwrap();
        let log = setup_test_logger();
        let base_dir = temp_dir.path();

        // Create test structure
        let code_path = base_dir.join("code.exe");
        let manifest_path = base_dir.join("code.VisualElementsManifest.xml");
        let commit_dir = base_dir.join("abc123");
        let bin_dir = base_dir.join("bin");
        let unins_file = base_dir.join("unins000.dat");
        let some_file = base_dir.join("somefile.txt");
		let other_dir = base_dir.join("otherdir");

        // Create files and directories
        fs::write(&code_path, "executable content").unwrap();
        fs::write(&manifest_path, "manifest content").unwrap();
        fs::create_dir(&commit_dir).unwrap();
        fs::write(commit_dir.join("commit_file.txt"), "commit content").unwrap();
        fs::create_dir(&bin_dir).unwrap();
        fs::write(bin_dir.join("old_binary.exe"), "old binary").unwrap();
        fs::write(bin_dir.join("new_binary.exe"), "new binary").unwrap();
        fs::write(&unins_file, "uninstall data").unwrap();
        fs::write(&some_file, "some file content").unwrap();
		fs::create_dir(&other_dir).unwrap();
        fs::write(other_dir.join("other_file.txt"), "other content").unwrap();

        // Perform the remove operation
        let result = remove_files(&log, &code_path, "abc123");

        assert!(result.is_ok(), "Remove operation should succeed");
        assert!(code_path.exists(), "Code executable should be preserved");
        assert!(manifest_path.exists(), "VisualElementsManifest.xml should be preserved");
        assert!(commit_dir.exists(), "Commit directory should be preserved");
        assert!(commit_dir.join("commit_file.txt").exists(), "Files in commit dir should be preserved");
        assert!(unins_file.exists(), "Unins files should be preserved");
		assert!(bin_dir.join("new_binary.exe").exists(), "Non-old files in bin should be preserved");


        assert!(!some_file.exists(), "Random files should be deleted");
        assert!(!bin_dir.join("old_binary.exe").exists(), "Old files in bin should be deleted");
		assert!(!other_dir.exists(), "Other directories should be deleted");
    }

    #[test]
    fn test_cleanup_dll_files_with_ffmpeg() {
        let temp_dir = tempdir().unwrap();
        let log = setup_test_logger();
        let base_dir = temp_dir.path();

        // Create test structure
        let code_path = base_dir.join("code.exe");
        let ffmpeg_dll = base_dir.join("ffmpeg.dll");
        let libcrypto_dll = base_dir.join("libcrypto.dll");
        let libssl_dll = base_dir.join("libssl.dll");
        let some_txt_file = base_dir.join("readme.txt");

        // Create files
        fs::write(&code_path, "executable content").unwrap();
        fs::write(&ffmpeg_dll, "ffmpeg library").unwrap();
        fs::write(&libcrypto_dll, "crypto library").unwrap();
        fs::write(&libssl_dll, "ssl library").unwrap();
        fs::write(&some_txt_file, "readme content").unwrap();

        // Perform cleanup
        let result = cleanup_dll_files(&log, &code_path);

        assert!(result.is_ok(), "Cleanup operation should succeed");
        assert!(code_path.exists(), "Code executable should be preserved");
        assert!(some_txt_file.exists(), "Non-DLL files should be preserved");
        assert!(!ffmpeg_dll.exists(), "ffmpeg.dll should be deleted");
        assert!(!libcrypto_dll.exists(), "libcrypto.dll should be deleted");
        assert!(!libssl_dll.exists(), "libssl.dll should be deleted");
    }

    #[test]
    fn test_cleanup_dll_files_without_ffmpeg() {
        let temp_dir = tempdir().unwrap();
        let log = setup_test_logger();
        let base_dir = temp_dir.path();

        // Create test structure without ffmpeg.dll
        let code_path = base_dir.join("code.exe");
        let some_dll = base_dir.join("somelibrary.dll");

        // Create files
        fs::write(&code_path, "executable content").unwrap();
        fs::write(&some_dll, "some library").unwrap();

        // Perform cleanup
        let result = cleanup_dll_files(&log, &code_path);

        assert!(result.is_ok(), "Cleanup operation should succeed");
        assert!(code_path.exists(), "Code executable should be preserved");
        assert!(some_dll.exists(), "DLL files should be preserved when ffmpeg.dll is not present");
    }

    #[test]
    fn test_cleanup_dll_files_case_insensitive() {
        let temp_dir = tempdir().unwrap();
        let log = setup_test_logger();
        let base_dir = temp_dir.path();

        // Create test structure with mixed case DLL extensions
        let code_path = base_dir.join("code.exe");
        let ffmpeg_dll = base_dir.join("ffmpeg.dll");
        let upper_dll = base_dir.join("LIBRARY.DLL");
        let mixed_dll = base_dir.join("another.Dll");

        // Create files
        fs::write(&code_path, "executable content").unwrap();
        fs::write(&ffmpeg_dll, "ffmpeg library").unwrap();
        fs::write(&upper_dll, "upper case dll").unwrap();
        fs::write(&mixed_dll, "mixed case dll").unwrap();

        // Perform cleanup
        let result = cleanup_dll_files(&log, &code_path);

        assert!(result.is_ok(), "Cleanup operation should succeed");
        assert!(!ffmpeg_dll.exists(), "ffmpeg.dll should be deleted");
        assert!(!upper_dll.exists(), "LIBRARY.DLL should be deleted (case insensitive)");
        assert!(!mixed_dll.exists(), "another.Dll should be deleted (case insensitive)");
    }
}

fn main() {
	let args: Vec<String> = env::args().collect();
	let mut log_path = env::temp_dir();
	log_path.push(format!(
		"vscode-inno-updater-{:?}.log",
		SystemTime::now()
			.duration_since(SystemTime::UNIX_EPOCH)
			.unwrap()
			.as_secs()
	));

	if args.len() == 3 && args[1] == "--parse" {
		let path = PathBuf::from(&args[2]);
		parse(&path).unwrap_or_else(|err| {
			eprintln!("{}", err);
			std::process::exit(1);
		});
	} else if args.len() == 4 && args[1] == "--gc" {
		let code_path = PathBuf::from(&args[2]);
		let commit_to_preserve = &args[3];

		if !code_path.is_absolute() {
			eprintln!("Error: Code path needs to be absolute. Instead got: {}", args[2]);
			std::process::exit(1);
		}

		if !code_path.exists() {
			eprintln!("Error: Code path doesn't seem to exist: {}", args[2]);
			std::process::exit(1);
		}

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

		info!(
			log,
			"Removing files from base directory of {:?}, preserving commit folder: {}", code_path, commit_to_preserve
		);

		remove_files(&log, &code_path, commit_to_preserve).unwrap_or_else(|err| {
			eprintln!("Error during file removal: {}", err);
			std::process::exit(1);
		});

		info!(log, "Successfully completed file removal operation");
	} else if args.len() == 4 && args[1] == "--update" {
		let uninstdat_path = PathBuf::from(&args[2]);
		let update_path = PathBuf::from(&args[3]);

		let decorator = slog_term::TermDecorator::new().build();
		let drain = slog_term::FullFormat::new(decorator).build().fuse();
		let drain = slog_async::Async::new(drain).build().fuse();
		let log = slog::Logger::root(drain, o!());

		info!(
			log,
			"Updating uninstall file {:?}, update path {:?}", uninstdat_path, update_path
		);

		patch_uninstdat(&log, &uninstdat_path, &update_path).unwrap_or_else(|err| {
			eprintln!("{}", err);
			std::process::exit(1);
		});

		info!(
			log,
			"Successfully updated uninstall file {:?}", uninstdat_path
		);
	} else if args.len() >= 3 && args[1] == "--gui" {
		let (tx, rx) = mpsc::channel();
		let label = args[2].clone();

		thread::spawn(move || {
			gui::run_progress_window(false, tx, label);
		});

		let window = rx.recv().unwrap();
		let duration = args.get(3).and_then(|v| v.parse().ok()).unwrap_or(5); // Default to 5 seconds if parsing fails

		window.update_status("Performing operation...");
		thread::sleep(std::time::Duration::from_secs(1));

		window.update_status("Processing files...");
		thread::sleep(std::time::Duration::from_secs(1));

		window.update_status("Almost done...");
		thread::sleep(std::time::Duration::from_secs(duration));
		window.exit();
	} else if args.len() == 3 && args[1] == "--retry-simulation" {
		let (tx, rx) = mpsc::channel();
		let label = args[2].clone();

		thread::spawn(move || {
			gui::run_progress_window(false, tx, label);
		});

		let window = rx.recv().unwrap();
		let result = util::retry(
			"simulating a failed retry operation",
			|_| -> Result<u32, Box<dyn error::Error>> {
				Err(Box::new(std::io::Error::new(
					std::io::ErrorKind::Other,
					"[[Simulated error message]]",
				)))
			},
			Some(5),
		);

		if result.is_err() {
			handle_error(log_path.to_str().unwrap());
		}

		window.exit();
	} else if args.len() == 3 && args[1] == "--error" {
		handle_error(log_path.to_str().unwrap());
	} else if args.len() == 2 && args[1] == "--crash" {
		panic!("Simulated crash");
	} else if args.len() == 2 && (args[1] == "--version" || args[1] == "-v") {
		eprintln!("Inno Update v{}", VERSION);
	} else {
		let args: Vec<String> = args.into_iter().filter(|a| !a.starts_with("--")).collect();
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

		if args.len() < 4 {
			eprintln!("Inno Update v{}", VERSION);
			eprintln!("Error: Bad usage");
			std::process::exit(1);
		} else {
			match _main(&log, &args) {
				Ok(_) => {
					info!(log, "Update was successful!");
					std::process::exit(0);
				}
				Err(err) => {
					error!(log, "{}", err);
					handle_error(log_path.to_str().unwrap());
					std::process::exit(1);
				}
			}
		}
	}
}
