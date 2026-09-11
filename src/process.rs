/*-----------------------------------------------------------------------------------------
 *  Copyright (c) Microsoft Corporation. All rights reserved.
 *  Licensed under the MIT License. See LICENSE in the project root for license information.
 *----------------------------------------------------------------------------------------*/

use std::ffi::c_void;
use std::path::{Path, PathBuf};
use std::{error, io, mem, ptr, thread, time};
use crate::strings::from_utf16;
use crate::util;

pub struct RunningProcess {
	pub name: String,
	pub id: u32,
}

#[derive(Debug)]
pub struct CapturedProcess {
	name: String,
	id: u32,
	handle: isize,
}

impl CapturedProcess {
	fn is_running(&self) -> Result<bool, Box<dyn error::Error>> {
		use windows_sys::Win32::Foundation::{WAIT_FAILED, WAIT_OBJECT_0, WAIT_TIMEOUT};
		use windows_sys::Win32::System::Threading::WaitForSingleObject;

		let wait_result = unsafe { WaitForSingleObject(self.handle, 0) };
		if wait_result == WAIT_TIMEOUT {
			Ok(true)
		} else if wait_result == WAIT_OBJECT_0 {
			Ok(false)
		} else if wait_result == WAIT_FAILED {
			Err(io::Error::new(
				io::ErrorKind::Other,
				format!(
					"Failed checking process {} state: {}",
					self.id,
					last_error_message()
				),
			)
			.into())
		} else {
			Err(io::Error::new(
				io::ErrorKind::Other,
				format!(
					"Unexpected wait result {} for process {}",
					wait_result, self.id
				),
			)
			.into())
		}
	}

	fn terminate_and_wait(&self, log: &slog::Logger) -> Result<(), Box<dyn error::Error>> {
		use windows_sys::Win32::Foundation::{WAIT_FAILED, WAIT_OBJECT_0, WAIT_TIMEOUT};
		use windows_sys::Win32::System::Threading::{TerminateProcess, WaitForSingleObject};

		if !self.is_running()? {
			info!(log, "{}, pid {} has already exited", self.name, self.id);
			return Ok(());
		}

		info!(
			log,
			"Found {} running, pid {}, attempting to kill...", self.name, self.id
		);

		if unsafe { TerminateProcess(self.handle, 0) } == 0 {
			return Err(io::Error::new(
				io::ErrorKind::Other,
				format!("Failed to kill process {}: {}", self.id, last_error_message()),
			)
			.into());
		}

		info!(
			log,
			"Termination requested for {}, pid {}, waiting for exit",
			self.name,
			self.id
		);

		let wait_result = unsafe { WaitForSingleObject(self.handle, 30_000) };
		if wait_result == WAIT_OBJECT_0 {
			info!(
				log,
				"Confirmed {}, pid {} has exited", self.name, self.id
			);
			Ok(())
		} else if wait_result == WAIT_TIMEOUT {
			Err(io::Error::new(
				io::ErrorKind::TimedOut,
				format!("Timed out waiting for process {} to exit", self.id),
			)
			.into())
		} else if wait_result == WAIT_FAILED {
			Err(io::Error::new(
				io::ErrorKind::Other,
				format!(
					"Failed waiting for process {} to exit: {}",
					self.id,
					last_error_message()
				),
			)
			.into())
		} else {
			Err(io::Error::new(
				io::ErrorKind::Other,
				format!(
					"Unexpected wait result {} for process {}",
					wait_result, self.id
				),
			)
			.into())
		}
	}
}

impl Drop for CapturedProcess {
	fn drop(&mut self) {
		use windows_sys::Win32::Foundation::CloseHandle;

		unsafe {
			CloseHandle(self.handle);
		}
	}
}

fn last_error_message() -> String {
	util::get_last_error_message().unwrap_or_else(|_| "unknown error".to_string())
}

fn paths_equal(first: &Path, second: &Path) -> bool {
	first
		.to_string_lossy()
		.eq_ignore_ascii_case(&second.to_string_lossy())
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

fn capture_process(
	log: &slog::Logger,
	process: &RunningProcess,
	path: &Path,
) -> Result<Option<CapturedProcess>, Box<dyn error::Error>> {
	use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, ERROR_INVALID_PARAMETER, MAX_PATH};
	use windows_sys::Win32::System::ProcessStatus::K32GetModuleFileNameExW;
	use windows_sys::Win32::System::Threading::{
		OpenProcess, WaitForSingleObject, PROCESS_QUERY_INFORMATION, PROCESS_TERMINATE,
		PROCESS_VM_READ,
	};

	const SYNCHRONIZE_ACCESS: u32 = 0x00100000;

	unsafe {
		let handle = OpenProcess(
			PROCESS_QUERY_INFORMATION | PROCESS_VM_READ | PROCESS_TERMINATE | SYNCHRONIZE_ACCESS,
			0,
			process.id,
		);

		if ptr::eq(handle as *mut c_void, ptr::null()) {
			if GetLastError() == ERROR_INVALID_PARAMETER {
				info!(log, "{}, pid {} exited before it could be captured", process.name, process.id);
				return Ok(None);
			}

			return Err(io::Error::new(
				io::ErrorKind::Other,
				format!(
					"Failed to open process {}: {}",
					process.id,
					last_error_message()
				),
			)
			.into());
		}

		let mut raw_path = [0u16; MAX_PATH as usize];
		let len = K32GetModuleFileNameExW(handle, mem::zeroed(), raw_path.as_mut_ptr(), MAX_PATH)
			as usize;

		if len == 0 {
			let process_exited = WaitForSingleObject(handle, 0) == 0;
			let message = last_error_message();
			CloseHandle(handle);

			if process_exited {
				info!(log, "{}, pid {} exited before its path could be verified", process.name, process.id);
				return Ok(None);
			}

			return Err(io::Error::new(
				io::ErrorKind::Other,
				format!(
					"Failed to get process {} file name: {}",
					process.id,
					message
				),
			)
			.into());
		}

		let process_path = PathBuf::from(from_utf16(&raw_path[0..len])?);

		info!(
			log,
			"Found {} running as pid {}", process_path.display(), process.id
		);

		if !paths_equal(&process_path, path) {
			info!(
				log,
				"Skipping pid {} because its path does not match the update target",
				process.id
			);
			CloseHandle(handle);
			return Ok(None);
		}

		Ok(Some(CapturedProcess {
			name: process.name.clone(),
			id: process.id,
			handle,
		}))
	}
}

pub fn capture_running_processes(
	log: &slog::Logger,
	path: &Path,
) -> Result<Vec<CapturedProcess>, Box<dyn error::Error>> {
	let file_name = path
		.file_name()
		.ok_or_else(|| io::Error::new(io::ErrorKind::Other, "Could not get process file name"))?
		.to_str()
		.ok_or_else(|| {
			io::Error::new(
				io::ErrorKind::Other,
				"Could not convert process file name to str",
			)
		})?;

	let target_processes = get_running_processes()?
		.into_iter()
		.filter(|process| process.name.eq_ignore_ascii_case(file_name))
		.filter_map(|process| capture_process(log, &process, path).transpose())
		.collect::<Result<Vec<_>, _>>()?;

	if target_processes.is_empty() {
		info!(log, "{} is not running", file_name);
	} else {
		info!(
			log,
			"Captured {} running {} processes: {:?}",
			target_processes.len(),
			file_name,
			target_processes.iter().map(|process| process.id).collect::<Vec<_>>()
		);
	}

	Ok(target_processes)
}

pub fn wait_or_kill(
	log: &slog::Logger,
	target_processes: &[CapturedProcess],
) -> Result<(), Box<dyn error::Error>> {
	if target_processes.is_empty() {
		return Ok(());
	}

	let file_name = &target_processes[0].name;
	info!(
		log,
		"Waiting for {} captured {} processes to exit",
		target_processes.len(),
		file_name
	);

	let mut attempt: u32 = 0;
	let mut still_running: Vec<&CapturedProcess>;

	loop {
		attempt += 1;

		info!(
			log,
			"Checking if {} processes are still running... (attempt {})", file_name, attempt
		);

		still_running = Vec::new();
		for process in target_processes {
			if process.is_running()? {
				still_running.push(process);
			}
		}

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

	util::retry(
		"attempting to kill any running processes",
		|attempt| {
			info!(
				log,
				"Attempting to kill remaining processes... (attempt {})", attempt
			);

			let kill_errors: Vec<_> = still_running
				.iter()
				.filter_map(|process| process.terminate_and_wait(log).err())
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
		let processes = capture_running_processes(&log, &fake_path).unwrap();
		let result = wait_or_kill(&log, &processes);
		assert!(result.is_ok(), "Should succeed when no processes are running");
	}

	#[test]
	fn test_wait_or_kill_process_exits_naturally() {
		let log = setup_test_logger();
		let test_helper_path = get_test_helper_path();
		let mut child = start_test_process(&["run-for-duration", "5"]).expect("Failed to start test process");
		assert!(wait_for_process_start("test_helper.exe", 1000), "Test process should start and be visible");
		let processes = capture_running_processes(&log, &test_helper_path).unwrap();
		let result = wait_or_kill(&log, &processes);
		let _ = child.wait();
		assert!(result.is_ok(), "Should succeed when process exits naturally");
	}

	#[test]
	fn test_wait_or_kill_invalid_path() {
		let log = setup_test_logger();
		let path = PathBuf::from("");
		let result = capture_running_processes(&log, &path);
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
		let processes = capture_running_processes(&log, &test_helper).unwrap();
		let result = wait_or_kill(&log, &processes);
		let _ = child1.wait();
		let _ = child2.wait();
		assert!(result.is_ok(), "Should succeed when killing multiple processes");
	}

	#[test]
	fn test_wait_or_kill_process_after_executable_rename() {
		let log = setup_test_logger();
		let temp_dir = tempfile::tempdir().expect("Failed to create temp directory");
		let process_path = temp_dir.path().join("test_helper.exe");
		let renamed_process_path = temp_dir.path().join("old_test_helper.exe");
		std::fs::copy(get_test_helper_path(), &process_path).expect("Failed to copy test process");

		let mut child = Command::new(&process_path)
			.arg("run-forever")
			.spawn()
			.expect("Failed to start test process");
		assert!(wait_for_process_start("test_helper.exe", 1000), "Test process should start and be visible");
		let processes = capture_running_processes(&log, &process_path).unwrap();
		assert_eq!(processes.len(), 1, "Should capture the test process before renaming");

		std::fs::rename(&process_path, &renamed_process_path).expect("Failed to rename running test process");
		let result = wait_or_kill(&log, &processes);
		let _ = child.wait();
		assert!(result.is_ok(), "Should kill and await the captured process after its executable is renamed");
	}
}
