/*-----------------------------------------------------------------------------------------
 *  Copyright (c) Microsoft Corporation. All rights reserved.
 *  Licensed under the MIT License. See LICENSE in the project root for license information.
 *----------------------------------------------------------------------------------------*/

use std::ffi::c_void;
use std::path::{Path, PathBuf};
use std::{error, io, mem, ptr, thread, time};
use strings::from_utf16;
use {slog, util};

pub struct RunningProcess {
	pub name: String,
	pub id: u32,
}

pub fn get_running_processes() -> Result<Vec<RunningProcess>, io::Error> {
	use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
	use windows_sys::Win32::System::Diagnostics::ToolHelp::{
		CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
		TH32CS_SNAPPROCESS,
	};

	unsafe {
		let handle = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);

		if handle == INVALID_HANDLE_VALUE {
			return Err(io::Error::new(
				io::ErrorKind::Other,
				"Could not create process snapshot",
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

		if Process32FirstW(handle, &mut pe32).is_negative() {
			CloseHandle(handle);

			return Err(io::Error::new(
				io::ErrorKind::Other,
				"Could not get first process data",
			));
		}

		let mut result: Vec<RunningProcess> = vec![];

		loop {
			result.push(RunningProcess {
				name: from_utf16(&pe32.szExeFile).map_err(|e| {
					CloseHandle(handle);
					e
				})?,
				id: pe32.th32ProcessID,
			});

			if Process32NextW(handle, &mut pe32).is_negative() {
				CloseHandle(handle);
				break;
			}
		}

		Ok(result)
	}
}

/**
 * Kills a running process, if its path is the same as the provided one.
 */
fn kill_process_if(
	log: &slog::Logger,
	process: &RunningProcess,
	path: &Path,
) -> Result<(), Box<dyn error::Error>> {
	use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, MAX_PATH};
	use windows_sys::Win32::System::ProcessStatus::K32GetModuleFileNameExW;
	use windows_sys::Win32::System::Threading::{
		OpenProcess, TerminateProcess, PROCESS_QUERY_INFORMATION, PROCESS_TERMINATE,
		PROCESS_VM_READ,
	};

	info!(
		log,
		"Kill process if found: {}, {}", process.id, process.name
	);

	unsafe {
		// https://msdn.microsoft.com/en-us/library/windows/desktop/ms684320(v=vs.85).aspx
		let handle: HANDLE = OpenProcess(
			PROCESS_QUERY_INFORMATION | PROCESS_VM_READ | PROCESS_TERMINATE,
			0,
			process.id,
		);

		if ptr::eq(handle as *mut c_void, ptr::null()) {
			return Err(io::Error::new(
				io::ErrorKind::Other,
				format!(
					"Failed to open process: {}",
					util::get_last_error_message()?
				),
			)
			.into());
		}

		let mut raw_path = [0u16; MAX_PATH as usize];
		let len = K32GetModuleFileNameExW(handle, mem::zeroed(), raw_path.as_mut_ptr(), MAX_PATH)
			as usize;

		if len == 0 {
			CloseHandle(handle);

			return Err(io::Error::new(
				io::ErrorKind::Other,
				format!(
					"Failed to get process file name: {}",
					util::get_last_error_message()?
				),
			)
			.into());
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

		if TerminateProcess(handle, 0).is_negative() {
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

pub fn wait_or_kill(log: &slog::Logger, path: &Path) -> Result<(), Box<dyn error::Error>> {
	let file_name = path
		.file_name()
		.ok_or_else(|| io::Error::new(io::ErrorKind::Other, "Could not get process file name"))?;

	let file_name = file_name.to_str().ok_or_else(|| {
		io::Error::new(
			io::ErrorKind::Other,
			"Could not get convert file name to str",
		)
	})?;

	let mut attempt: u32 = 0;

	// wait for 10 seconds until all processes are dead
	loop {
		attempt += 1;

		info!(
			log,
			"Checking for running {} processes... (attempt {})", file_name, attempt
		);

		let process_found = get_running_processes()?
			.into_iter()
			.any(|p| p.name == file_name);

		if !process_found {
			info!(log, "{} is not running", file_name);
			break;
		}

		// give up after 60 * 500ms = 30 seconds
		if attempt == 60 {
			info!(log, "Gave up waiting for {} to exit", file_name);
			break;
		}

		info!(log, "{} is running, wait a bit", file_name);
		thread::sleep(time::Duration::from_millis(500));
	}

	// try to kill any running processes
	util::retry(
		"attempting to kill any running Code.exe processes",
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
