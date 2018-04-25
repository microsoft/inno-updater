/*-----------------------------------------------------------------------------------------
 *  Copyright (c) Microsoft Corporation. All rights reserved.
 *  Licensed under the MIT License. See LICENSE in the project root for license information.
 *----------------------------------------------------------------------------------------*/

use std::{error, io, mem, ptr, thread, time};
use std::path::{Path, PathBuf};
use winapi::shared::minwindef::{DWORD, TRUE};
use strings::from_utf16;
use util;
use slog;

pub struct RunningProcess {
	pub name: String,
	pub id: DWORD,
}

pub fn get_running_processes() -> Result<Vec<RunningProcess>, io::Error> {
	use winapi::um::handleapi::{CloseHandle, INVALID_HANDLE_VALUE};
	use winapi::um::tlhelp32::{CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW,
	                           Process32NextW, TH32CS_SNAPPROCESS};

	unsafe {
		let handle = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);

		if handle == INVALID_HANDLE_VALUE {
			return Err(io::Error::new(
				io::ErrorKind::Other,
				"could not create process snapshot",
			));
		}

		let mut pe32 = PROCESSENTRY32W {
			dwSize: 0,
			cntUsage: 0,
			th32ProcessID: 0,
			th32DefaultHeapID: 0,
			th32ModuleID: 0,
			cntThreads: 0,
			th32ParentProcessID: 0,
			pcPriClassBase: 0,
			dwFlags: 0,
			szExeFile: [0u16; 260],
		};

		pe32.dwSize = mem::size_of::<PROCESSENTRY32W>() as u32;

		if Process32FirstW(handle, &mut pe32) != TRUE {
			CloseHandle(handle);

			return Err(io::Error::new(
				io::ErrorKind::Other,
				"could not get first process data",
			));
		}

		let mut result: Vec<RunningProcess> = vec![];

		loop {
			result.push(RunningProcess {
				name: from_utf16(&pe32.szExeFile)?,
				id: pe32.th32ProcessID,
			});

			if Process32NextW(handle, &mut pe32) != TRUE {
				CloseHandle(handle);
				break;
			}
		}

		return Ok(result);
	}
}

fn get_last_error_message() -> Result<String, Box<error::Error>> {
	use winapi::um::errhandlingapi::GetLastError;
	use winapi::um::winbase::{FormatMessageW, FORMAT_MESSAGE_FROM_SYSTEM,
	                          FORMAT_MESSAGE_IGNORE_INSERTS};

	let mut error_message = [0u16; 32000];
	let error_message_len: usize;

	unsafe {
		error_message_len = FormatMessageW(
			FORMAT_MESSAGE_FROM_SYSTEM | FORMAT_MESSAGE_IGNORE_INSERTS,
			ptr::null_mut(),
			GetLastError(),
			0,
			error_message.as_mut_ptr(),
			32000,
			ptr::null_mut(),
		) as usize;
	}

	Ok(match error_message_len {
		0 => String::from("unknown error"),
		_ => from_utf16(&error_message[0..error_message_len])?,
	})
}

/**
 * Kills a running process, if its path is the same as the provided one.
 */
fn kill_process_if(
	log: &slog::Logger,
	process: &RunningProcess,
	path: &Path,
) -> Result<(), Box<error::Error>> {
	use winapi::shared::minwindef::MAX_PATH;
	use winapi::um::processthreadsapi::{OpenProcess, TerminateProcess};
	use winapi::um::psapi::GetModuleFileNameExW;
	use winapi::um::handleapi::CloseHandle;
	use winapi::um::winnt::{PROCESS_QUERY_INFORMATION, PROCESS_TERMINATE, PROCESS_VM_READ};

	info!(
		log,
		"Kill process if found: {}, {}", process.id, process.name
	);

	unsafe {
		// https://msdn.microsoft.com/en-us/library/windows/desktop/ms684320(v=vs.85).aspx
		let handle = OpenProcess(
			PROCESS_QUERY_INFORMATION | PROCESS_VM_READ | PROCESS_TERMINATE,
			0,
			process.id,
		);

		if handle == ptr::null_mut() {
			return Err(
				io::Error::new(
					io::ErrorKind::Other,
					format!("Failed to open process: {}", get_last_error_message()?),
				).into(),
			);
		}

		let mut raw_path = [0u16; MAX_PATH];
		let len = GetModuleFileNameExW(
			handle,
			ptr::null_mut(),
			raw_path.as_mut_ptr(),
			MAX_PATH as DWORD,
		) as usize;

		if len == 0 {
			CloseHandle(handle);

			return Err(
				io::Error::new(
					io::ErrorKind::Other,
					format!(
						"Failed to get process file name: {}",
						get_last_error_message()?
					),
				).into(),
			);
		}

		let process_path = PathBuf::from(from_utf16(&raw_path[0..len])?);

		if process_path != path {
			CloseHandle(handle);
			return Ok(());
		}

		info!(
			log,
			"Found {} running, pid {}, attempting to kill...", process.name, process.id
		);

		if TerminateProcess(handle, 0) != TRUE {
			return Err(io::Error::new(io::ErrorKind::Other, "Failed to kill process").into());
		}

		info!(
			log,
			"Successfully killed {}, pid {}", process.name, process.id
		);

		CloseHandle(handle);
		Ok(())
	}
}

pub fn wait_or_kill(log: &slog::Logger, path: &Path) -> Result<(), Box<error::Error>> {
	let file_name = path.file_name().ok_or(io::Error::new(
		io::ErrorKind::Other,
		"could not get process file name",
	))?;

	let file_name = file_name.to_str().ok_or(io::Error::new(
		io::ErrorKind::Other,
		"could not get convert file name to str",
	))?;

	let mut attempt: u32 = 0;

	// wait for 10 seconds until all processes are dead
	loop {
		attempt += 1;

		info!(
			log,
			"Checking for running {} processes... (attempt {})", file_name, attempt
		);

		let processes: Vec<_> = get_running_processes()?
			.into_iter()
			.filter(|p| p.name == file_name)
			.collect();

		if processes.len() == 0 {
			info!(log, "{} is not running", file_name);
			break;
		}

		// give up after 60 * 500ms = 30 seconds
		if attempt == 60 || processes.len() == 0 {
			info!(log, "Gave up waiting for {} to exit", file_name);
			break;
		}

		info!(log, "{} is running, wait a bit", file_name);
		thread::sleep(time::Duration::from_millis(500));
	}

	// try to kill any running processes
	util::retry(
		|attempt| {
			info!(
				log,
				"Checking for possible conflicting running processes... (attempt {})", attempt
			);

			let kill_errors: Vec<_> = get_running_processes()?
				.into_iter()
				.filter(|p| p.name == file_name)
				.filter_map(|p| kill_process_if(log, &p, path).err())
				.collect();

			for err in &kill_errors {
				warn!(log, "Kill error {}", err);
			}

			match kill_errors.len() {
				0 => Ok(()),
				_ => Err(kill_errors.into_iter().nth(1).unwrap()),
			}
		},
		None,
	)
}
