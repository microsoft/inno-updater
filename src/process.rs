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

		if Process32FirstW(handle, &mut pe32) == 0 {
			CloseHandle(handle);

			return Err(io::Error::new(
				io::ErrorKind::Other,
				"Could not get first process data",
			));
		}

		let mut result: Vec<RunningProcess> = vec![];

		loop {
			result.push(RunningProcess {
				name: from_utf16(&pe32.szExeFile).inspect_err(|_| {
					CloseHandle(handle);
				})?,
				id: pe32.th32ProcessID,
			});

			if Process32NextW(handle, &mut pe32) == 0 {
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
	use windows_sys::Win32::Foundation::{CloseHandle, MAX_PATH, ERROR_ACCESS_DENIED, GetLastError};
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
		let handle = OpenProcess(
			PROCESS_QUERY_INFORMATION | PROCESS_VM_READ | PROCESS_TERMINATE,
			0,
			process.id,
		);

	        if ptr::eq(handle as *mut c_void, ptr::null_mut()) {
	            let error_code = GetLastError();
	
	            // Check for insufficient permission
	            if error_code == ERROR_ACCESS_DENIED {
	                info!(
	                    log,
	                    "Insufficient permissions to open process: {}", process.id
	                );
	                return Ok(()); // Ignore the error and return Ok
	            } else {
	                return Err(io::Error::new(
	                    io::ErrorKind::Other,
	                    format!(
	                        "Failed to open process: {}",
	                        util::get_last_error_message()?
	                    ),
	                ).into());
	            }
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

		info!(
			log,
			"Found {} running {}, attempting to kill...", process_path.display(), path.display()
		);

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

/**
 * Checks if a process with the given PID is still running.
 */
fn is_process_running(pid: u32) -> bool {
	use std::ffi::c_void;
	use windows_sys::Win32::Foundation::CloseHandle;
	use windows_sys::Win32::System::Threading::{
		GetExitCodeProcess, OpenProcess, PROCESS_QUERY_INFORMATION,
	};

	const STILL_ACTIVE: u32 = 259;

	unsafe {
		let handle = OpenProcess(PROCESS_QUERY_INFORMATION, 0, pid);

		if ptr::eq(handle as *mut c_void, ptr::null()) {
			return false;
		}

		let mut exit_code = 0u32;
		let result = GetExitCodeProcess(handle, &mut exit_code);
		CloseHandle(handle);

		result != 0 && exit_code == STILL_ACTIVE
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

	// Get the initial list of processes that match our target
	let target_processes: Vec<RunningProcess> = get_running_processes()?
		.into_iter()
		.filter(|p| p.name == file_name)
		.collect();

	if target_processes.is_empty() {
		info!(log, "{} is not running", file_name);
		return Ok(());
	}

	info!(
		log,
		"Found {} running {} processes: {:?}",
		target_processes.len(),
		file_name,
		target_processes.iter().map(|p| p.id).collect::<Vec<_>>()
	);

	let mut attempt: u32 = 0;
	let mut still_running: Vec<&RunningProcess>;

	// wait for up to 30 seconds until all target processes are dead
	loop {
		attempt += 1;

		info!(
			log,
			"Checking if {} processes are still running... (attempt {})", file_name, attempt
		);

		still_running = target_processes
			.iter()
			.filter(|p| is_process_running(p.id))
			.collect();

		if still_running.is_empty() {
			info!(log, "All {} processes have exited", file_name);
			break;
		}

		// give up after 60 * 500ms = 30 seconds
		if attempt == 60 {
			info!(
				log,
				"Gave up waiting for {} to exit, {} processes still running: {:?}",
				file_name,
				still_running.len(),
				still_running.iter().map(|p| p.id).collect::<Vec<_>>()
			);
			break;
		}

		info!(
			log,
			"{} processes still running: {:?}, waiting...",
			still_running.len(),
			still_running.iter().map(|p| p.id).collect::<Vec<_>>()
		);
		thread::sleep(time::Duration::from_millis(500));
	}

	// try to kill any running target processes
	util::retry(
		"attempting to kill any running processes",
		|attempt| {
			info!(
				log,
				"Attempting to kill remaining processes... (attempt {})", attempt
			);

			let kill_errors: Vec<_> = still_running
				.iter()
				.filter_map(|p| kill_process_if(log, p, path).err())
				.collect();

			for err in &kill_errors {
				warn!(log, "Kill error {}", err);
			}

			match kill_errors.len() {
				0 => Ok(()),
				_ => Err(kill_errors.into_iter().nth(0).unwrap()),
			}
		},
		None,
	)
}

#[cfg(test)]
mod tests {
	use super::*;
	use std::path::PathBuf;
	use std::process::{Command, Child};
	use std::thread;
	use std::time::Duration;
	use slog::{Logger, o, Drain};
	use slog_term::{TermDecorator, FullFormat};
	use slog_async::Async;

	fn setup_test_logger() -> Logger {
		let decorator = TermDecorator::new().build();
		let drain = FullFormat::new(decorator).build().fuse();
		let drain = Async::new(drain).build().fuse();
		Logger::root(drain, o!())
	}

	fn get_test_helper_path() -> PathBuf {
		let profile = std::env::var("PROFILE").unwrap_or_else(|_| "debug".to_string());
		let target_dir = std::env::var("CARGO_TARGET_DIR").unwrap_or_else(|_| "target".to_string());
		let target = std::env::var("TARGET").unwrap_or_else(|_| {
			"i686-pc-windows-msvc".to_string()
		});

		// Resolve target_dir to absolute path relative to project root
		let project_root = std::env::current_dir().expect("Failed to get current directory");
		let absolute_target_dir = project_root.join(&target_dir);
		absolute_target_dir
			.join(&target)
			.join(&profile)
			.join("test_helper.exe")
	}

	fn start_test_process(args: &[&str]) -> Result<Child, std::io::Error> {
		let test_helper = get_test_helper_path();
		Command::new(&test_helper)
			.args(args)
			.spawn()
	}

	fn wait_for_process_start(expected_name: &str, timeout_ms: u64) -> bool {
		let start = std::time::Instant::now();
		while start.elapsed().as_millis() < timeout_ms as u128 {
			if let Ok(processes) = get_running_processes() {
				if processes.iter().any(|p| p.name == expected_name) {
					return true;
				}
			}
			thread::sleep(Duration::from_millis(10));
		}
		false
	}

	#[test]
	fn test_wait_or_kill_no_processes_running() {
		let log = setup_test_logger();
		let fake_path = PathBuf::from("C:\\nonexistent\\fake_process.exe");
		let result = wait_or_kill(&log, &fake_path);
		assert!(result.is_ok(), "Should succeed when no processes are running");
	}

	#[test]
	fn test_wait_or_kill_process_exits_naturally() {
		let log = setup_test_logger();
		let test_helper_path = get_test_helper_path();
		let mut child = start_test_process(&["run-for-duration", "5"]).expect("Failed to start test process");
		assert!(wait_for_process_start("test_helper.exe", 1000), "Test process should start and be visible");
		let result = wait_or_kill(&log, &test_helper_path);
		let _ = child.wait();
		assert!(result.is_ok(), "Should succeed when process exits naturally");
	}

	#[test]
	fn test_wait_or_kill_invalid_path() {
		let log = setup_test_logger();
		let path = PathBuf::from("");
		let result = wait_or_kill(&log, &path);
		assert!(result.is_err(), "Should fail with invalid path");
		assert!(result.unwrap_err().to_string().contains("Could not get process file name"));
	}

	#[test]
	fn test_wait_or_kill_multiple_processes() {
		let log = setup_test_logger();
		let test_helper = get_test_helper_path();
		let mut child1 = start_test_process(&["run-forever"]).expect("Failed to start test process 1");
		let mut child2 = start_test_process(&["run-forever"]).expect("Failed to start test process 2");
		assert!(wait_for_process_start("test_helper.exe", 2000), "Test process should start and be visible");
		let processes = get_running_processes().unwrap();
		let test_helper_count = processes.iter().filter(|p| p.name == "test_helper.exe").count();
		assert!(test_helper_count >= 2, "Should have at least 2 test helper processes running");
		let result = wait_or_kill(&log, &test_helper);
		let _ = child1.wait();
		let _ = child2.wait();
		assert!(result.is_ok(), "Should succeed when killing multiple processes");
	}
}
