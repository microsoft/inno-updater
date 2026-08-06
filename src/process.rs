/*-----------------------------------------------------------------------------------------
 *  Copyright (c) Microsoft Corporation. All rights reserved.
 *  Licensed under the MIT License. See LICENSE in the project root for license information.
 *----------------------------------------------------------------------------------------*/

use crate::strings::{from_utf16, to_u16s};
use crate::util;
use std::ffi::OsString;
use std::os::windows::ffi::OsStringExt;
use std::path::{Path, PathBuf};
use std::{error, io, mem, thread, time};
use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE};

const MAX_PROCESS_PATH_LENGTH: usize = 32_768;

struct OwnedHandle(HANDLE);

impl OwnedHandle {
	fn new(handle: HANDLE) -> io::Result<Self> {
		if handle == 0 || handle == INVALID_HANDLE_VALUE {
			Err(io::Error::last_os_error())
		} else {
			Ok(Self(handle))
		}
	}

	fn get(&self) -> HANDLE {
		self.0
	}
}

impl Drop for OwnedHandle {
	fn drop(&mut self) {
		unsafe {
			CloseHandle(self.0);
		}
	}
}

struct MatchingProcess {
	process: RunningProcess,
	handle: OwnedHandle,
}

pub struct RunningProcess {
	pub name: String,
	pub id: u32,
}

pub fn get_running_processes() -> Result<Vec<RunningProcess>, io::Error> {
	use windows_sys::Win32::Foundation::ERROR_NO_MORE_FILES;
	use windows_sys::Win32::System::Diagnostics::ToolHelp::{
		CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW,
		TH32CS_SNAPPROCESS,
	};

	unsafe {
		let handle =
			OwnedHandle::new(CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0)).map_err(|err| {
				io::Error::new(
					err.kind(),
					format!("Could not create process snapshot: {}", err),
				)
			})?;

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

		if Process32FirstW(handle.get(), &mut pe32) == 0 {
			return Err(io::Error::other(format!(
				"Could not get first process data: {}",
				io::Error::last_os_error()
			)));
		}

		let mut result: Vec<RunningProcess> = vec![];

		loop {
			result.push(RunningProcess {
				name: from_utf16(&pe32.szExeFile)?,
				id: pe32.th32ProcessID,
			});

			if Process32NextW(handle.get(), &mut pe32) == 0 {
				let err = io::Error::last_os_error();
				if err.raw_os_error() == Some(ERROR_NO_MORE_FILES as i32) {
					break;
				}

				return Err(io::Error::new(
					err.kind(),
					format!("Could not get next process data: {}", err),
				));
			}
		}

		Ok(result)
	}
}

fn open_process(process_id: u32, access: u32) -> io::Result<OwnedHandle> {
	use windows_sys::Win32::System::Threading::OpenProcess;

	unsafe { OwnedHandle::new(OpenProcess(access, 0, process_id)) }
}

fn get_process_path_from_handle(handle: HANDLE) -> io::Result<PathBuf> {
	use windows_sys::Win32::System::Threading::QueryFullProcessImageNameW;

	unsafe {
		let mut raw_path = vec![0u16; MAX_PROCESS_PATH_LENGTH];
		let mut len = raw_path.len() as u32;
		if QueryFullProcessImageNameW(handle, 0, raw_path.as_mut_ptr(), &mut len) == 0 {
			return Err(io::Error::last_os_error());
		}

		raw_path.truncate(len as usize);
		Ok(PathBuf::from(OsString::from_wide(&raw_path)))
	}
}

#[cfg(test)]
fn get_process_path(process_id: u32) -> io::Result<PathBuf> {
	use windows_sys::Win32::System::Threading::PROCESS_QUERY_LIMITED_INFORMATION;

	let handle = open_process(process_id, PROCESS_QUERY_LIMITED_INFORMATION)?;
	get_process_path_from_handle(handle.get())
}

fn paths_equal(left: &Path, right: &Path) -> bool {
	use windows_sys::Win32::Globalization::{CSTR_EQUAL, CompareStringOrdinal};

	let left = to_u16s(left.as_os_str());
	let right = to_u16s(right.as_os_str());
	unsafe { CompareStringOrdinal(left.as_ptr(), -1, right.as_ptr(), -1, 1) == CSTR_EQUAL as i32 }
}

fn process_matches_target(
	process: &RunningProcess,
	process_path: &Path,
	target_path: &Path,
) -> bool {
	let Some(target_name) = target_path.file_name() else {
		return false;
	};
	let Some(target_parent) = target_path.parent() else {
		return false;
	};
	let Some(process_parent) = process_path.parent() else {
		return false;
	};

	// The image path follows an on-disk rename, while Toolhelp preserves the launch name.
	// The updater keeps old_Code.exe beside Code.exe, so match the launch name and directory.
	paths_equal(Path::new(&process.name), Path::new(target_name))
		&& paths_equal(process_parent, target_parent)
}

#[cfg(test)]
fn process_has_path(process_id: u32, path: &Path) -> io::Result<bool> {
	use windows_sys::Win32::Storage::FileSystem::SYNCHRONIZE;
	use windows_sys::Win32::System::Threading::PROCESS_QUERY_LIMITED_INFORMATION;

	let handle = open_process(process_id, PROCESS_QUERY_LIMITED_INFORMATION | SYNCHRONIZE)?;
	get_active_process_path(handle.get()).map(|process_path| {
		process_path.is_some_and(|process_path| paths_equal(&process_path, path))
	})
}

fn process_has_exited(handle: HANDLE) -> io::Result<bool> {
	use windows_sys::Win32::Foundation::{WAIT_FAILED, WAIT_OBJECT_0, WAIT_TIMEOUT};
	use windows_sys::Win32::System::Threading::WaitForSingleObject;

	unsafe {
		match WaitForSingleObject(handle, 0) {
			WAIT_OBJECT_0 => Ok(true),
			WAIT_TIMEOUT => Ok(false),
			WAIT_FAILED => Err(io::Error::last_os_error()),
			result => Err(io::Error::other(format!(
				"Unexpected process wait result: {}",
				result
			))),
		}
	}
}

fn get_active_process_path(handle: HANDLE) -> io::Result<Option<PathBuf>> {
	if process_has_exited(handle)? {
		return Ok(None);
	}

	match get_process_path_from_handle(handle) {
		Ok(path) => Ok(Some(path)),
		Err(_) if process_has_exited(handle)? => Ok(None),
		Err(err) => Err(err),
	}
}

fn wait_for_process_exit(handle: HANDLE, timeout: time::Duration) -> io::Result<()> {
	use windows_sys::Win32::Foundation::{WAIT_FAILED, WAIT_OBJECT_0, WAIT_TIMEOUT};
	use windows_sys::Win32::System::Threading::WaitForSingleObject;

	let timeout_ms = timeout.as_millis().min((u32::MAX - 1) as u128) as u32;
	unsafe {
		match WaitForSingleObject(handle, timeout_ms) {
			WAIT_OBJECT_0 => Ok(()),
			WAIT_TIMEOUT => Err(io::Error::new(
				io::ErrorKind::TimedOut,
				"Timed out waiting for process to exit",
			)),
			WAIT_FAILED => Err(io::Error::last_os_error()),
			result => Err(io::Error::other(format!(
				"Unexpected process wait result: {}",
				result
			))),
		}
	}
}

/**
 * Kills a running process if its path still matches the provided path.
 */
fn kill_process_if(
	log: &slog::Logger,
	matching_process: &MatchingProcess,
	path: &Path,
	exit_timeout: time::Duration,
) -> Result<(), Box<dyn error::Error>> {
	use windows_sys::Win32::Foundation::ERROR_INVALID_PARAMETER;
	use windows_sys::Win32::Storage::FileSystem::SYNCHRONIZE;
	use windows_sys::Win32::System::Threading::{
		PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_TERMINATE, TerminateProcess,
	};

	let process = &matching_process.process;
	info!(
		log,
		"Verifying process before termination: pid={}, name={}", process.id, process.name
	);

	if process_has_exited(matching_process.handle.get())? {
		info!(log, "Process {} has already exited", process.id);
		return Ok(());
	}

	let handle = match open_process(
		process.id,
		PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_TERMINATE | SYNCHRONIZE,
	) {
		Ok(handle) => handle,
		Err(err) if err.raw_os_error() == Some(ERROR_INVALID_PARAMETER as i32) => {
			info!(log, "Process {} has already exited", process.id);
			return Ok(());
		}
		Err(err) => {
			return Err(io::Error::new(
				err.kind(),
				format!(
					"Failed to open process {} for termination: {}",
					process.id, err
				),
			)
			.into());
		}
	};

	let Some(process_path) = get_active_process_path(handle.get()).map_err(|err| {
		io::Error::new(
			err.kind(),
			format!(
				"Failed to inspect process {} before termination: {}",
				process.id, err
			),
		)
	})?
	else {
		info!(log, "Process {} has already exited", process.id);
		return Ok(());
	};
	if !process_matches_target(process, &process_path, path) {
		info!(
			log,
			"Skipping pid {} because its path changed to {}",
			process.id,
			process_path.display()
		);
		return Ok(());
	}

	info!(
		log,
		"Terminating {}, pid {}",
		process_path.display(),
		process.id
	);
	unsafe {
		if TerminateProcess(handle.get(), 1) == 0 {
			let err = io::Error::last_os_error();
			return Err(io::Error::new(
				err.kind(),
				format!("Failed to terminate process {}: {}", process.id, err),
			)
			.into());
		}
	}

	wait_for_process_exit(handle.get(), exit_timeout).map_err(|err| {
		io::Error::new(
			err.kind(),
			format!("Failed waiting for process {} to exit: {}", process.id, err),
		)
	})?;
	info!(
		log,
		"Successfully terminated {}, pid {}", process.name, process.id
	);
	Ok(())
}

fn get_matching_processes(
	log: &slog::Logger,
	path: &Path,
) -> Result<Vec<MatchingProcess>, Box<dyn error::Error>> {
	use windows_sys::Win32::Storage::FileSystem::SYNCHRONIZE;
	use windows_sys::Win32::System::Threading::PROCESS_QUERY_LIMITED_INFORMATION;

	let file_name = path
		.file_name()
		.ok_or_else(|| io::Error::other("Could not get process file name"))?;

	let file_name = file_name.to_string_lossy();
	let mut target_processes = Vec::new();
	for process in get_running_processes()?
		.into_iter()
		.filter(|process| process.name.eq_ignore_ascii_case(&file_name))
	{
		let handle = match open_process(process.id, PROCESS_QUERY_LIMITED_INFORMATION | SYNCHRONIZE)
		{
			Ok(handle) => handle,
			Err(err) => {
				warn!(
					log,
					"Unable to inspect pid {} with the same name; skipping it: {}", process.id, err
				);
				continue;
			}
		};

		match get_active_process_path(handle.get()) {
			Ok(Some(process_path)) if process_matches_target(&process, &process_path, path) => {
				target_processes.push(MatchingProcess { process, handle });
			}
			Ok(Some(process_path)) => info!(
				log,
				"Ignoring pid {} with the same name at {}",
				process.id,
				process_path.display()
			),
			Ok(None) => {}
			Err(err) => warn!(
				log,
				"Unable to inspect pid {} with the same name; skipping it: {}", process.id, err
			),
		}
	}

	if target_processes.is_empty() {
		info!(log, "{} is not running", file_name);
	}

	Ok(target_processes)
}

fn wait_or_kill_with_options(
	log: &slog::Logger,
	path: &Path,
	max_wait_attempts: u32,
	wait_interval: time::Duration,
	exit_timeout: time::Duration,
) -> Result<(), Box<dyn error::Error>> {
	let file_name = path
		.file_name()
		.ok_or_else(|| io::Error::other("Could not get process file name"))?
		.to_string_lossy();
	let target_processes = get_matching_processes(log, path)?;
	if target_processes.is_empty() {
		return Ok(());
	}

	info!(
		log,
		"Found {} running {} processes at {}: {:?}",
		target_processes.len(),
		file_name,
		path.display(),
		target_processes
			.iter()
			.map(|process| process.process.id)
			.collect::<Vec<_>>()
	);

	let mut attempt: u32 = 0;
	let mut still_running: Vec<&MatchingProcess>;

	// Wait for the matching processes to exit naturally.
	loop {
		attempt += 1;

		info!(
			log,
			"Checking if {} processes are still running... (attempt {})", file_name, attempt
		);

		still_running = Vec::new();
		for process in &target_processes {
			if !process_has_exited(process.handle.get()).map_err(|err| {
				io::Error::new(
					err.kind(),
					format!("Failed waiting for process {}: {}", process.process.id, err),
				)
			})? {
				still_running.push(process);
			}
		}

		if still_running.is_empty() {
			info!(log, "All {} processes have exited", file_name);
			break;
		}

		if attempt >= max_wait_attempts {
			info!(
				log,
				"Gave up waiting for {} to exit, {} processes still running: {:?}",
				file_name,
				still_running.len(),
				still_running
					.iter()
					.map(|process| process.process.id)
					.collect::<Vec<_>>()
			);
			break;
		}

		info!(
			log,
			"{} processes still running: {:?}, waiting...",
			still_running.len(),
			still_running
				.iter()
				.map(|process| process.process.id)
				.collect::<Vec<_>>()
		);
		thread::sleep(wait_interval);
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
				.filter_map(|process| kill_process_if(log, process, path, exit_timeout).err())
				.collect();

			for err in &kill_errors {
				warn!(log, "Kill error {}", err);
			}

			match kill_errors.len() {
				0 => Ok(()),
				_ => Err(kill_errors.into_iter().next().unwrap()),
			}
		},
		None,
	)
}

pub fn wait_or_kill(log: &slog::Logger, path: &Path) -> Result<(), Box<dyn error::Error>> {
	wait_or_kill_with_options(
		log,
		path,
		60,
		time::Duration::from_millis(500),
		time::Duration::from_secs(2),
	)
}

#[cfg(test)]
mod tests {
	use super::*;
	use slog::{Drain, Logger, o};
	use slog_async::Async;
	use slog_term::{FullFormat, TermDecorator};
	use std::process::{Child, Command};
	use std::sync::Mutex;
	use std::thread;
	use std::time::Duration;

	static PROCESS_TEST_MUTEX: Mutex<()> = Mutex::new(());

	fn setup_test_logger() -> Logger {
		let decorator = TermDecorator::new().build();
		let drain = FullFormat::new(decorator).build().fuse();
		let drain = Async::new(drain).build().fuse();
		Logger::root(drain, o!())
	}

	fn get_test_helper_path() -> PathBuf {
		let profile = std::env::var("PROFILE").unwrap_or_else(|_| "debug".to_string());
		let target_dir = std::env::var("CARGO_TARGET_DIR").unwrap_or_else(|_| "target".to_string());
		let target = std::env::var("TARGET").unwrap_or_else(|_| "i686-pc-windows-msvc".to_string());

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
		start_test_process_at(&test_helper, args)
	}

	fn start_test_process_at(path: &Path, args: &[&str]) -> Result<Child, std::io::Error> {
		Command::new(path).args(args).spawn()
	}

	fn wait_for_process_path(process_id: u32, path: &Path, timeout_ms: u64) -> bool {
		let start = std::time::Instant::now();
		while start.elapsed().as_millis() < timeout_ms as u128 {
			if process_has_path(process_id, path).unwrap_or(false) {
				return true;
			}
			thread::sleep(Duration::from_millis(10));
		}
		false
	}

	fn wait_or_kill_for_test(log: &Logger, path: &Path) -> Result<(), Box<dyn error::Error>> {
		wait_or_kill_with_options(
			log,
			path,
			1,
			Duration::from_millis(10),
			Duration::from_secs(2),
		)
	}

	#[test]
	fn test_get_current_process_path() {
		let actual = get_process_path(std::process::id()).expect("Should get current process path");
		let expected = std::env::current_exe().expect("Should get current executable path");
		assert!(
			paths_equal(&actual, &expected),
			"Expected {:?}, got {:?}",
			expected,
			actual
		);
	}

	#[test]
	fn test_paths_equal_ignores_case() {
		assert!(paths_equal(
			Path::new("C:\\Program Files\\Microsoft VS Code\\Code.exe"),
			Path::new("c:\\program files\\microsoft vs code\\CODE.EXE"),
		));
	}

	#[test]
	fn test_wait_or_kill_no_processes_running() {
		let log = setup_test_logger();
		let fake_path = PathBuf::from("C:\\nonexistent\\fake_process.exe");
		let result = wait_or_kill(&log, &fake_path);
		assert!(
			result.is_ok(),
			"Should succeed when no processes are running"
		);
	}

	#[test]
	fn test_wait_or_kill_process_exits_naturally() {
		let _guard = PROCESS_TEST_MUTEX
			.lock()
			.unwrap_or_else(|err| err.into_inner());
		let log = setup_test_logger();
		let test_helper_path = get_test_helper_path();
		let mut child =
			start_test_process(&["run-for-duration", "1"]).expect("Failed to start test process");
		assert!(
			wait_for_process_path(child.id(), &test_helper_path, 1000),
			"Test process should start and be visible"
		);
		let result = wait_or_kill_with_options(
			&log,
			&test_helper_path,
			200,
			Duration::from_millis(10),
			Duration::from_secs(2),
		);
		let _ = child.wait();
		assert!(
			result.is_ok(),
			"Should succeed when process exits naturally"
		);
	}

	#[test]
	fn test_wait_or_kill_invalid_path() {
		let log = setup_test_logger();
		let path = PathBuf::from("");
		let result = wait_or_kill(&log, &path);
		assert!(result.is_err(), "Should fail with invalid path");
		assert!(
			result
				.unwrap_err()
				.to_string()
				.contains("Could not get process file name")
		);
	}

	#[test]
	fn test_wait_or_kill_multiple_processes() {
		let _guard = PROCESS_TEST_MUTEX
			.lock()
			.unwrap_or_else(|err| err.into_inner());
		let log = setup_test_logger();
		let test_helper = get_test_helper_path();
		let mut child1 =
			start_test_process(&["run-forever"]).expect("Failed to start test process 1");
		let mut child2 =
			start_test_process(&["run-forever"]).expect("Failed to start test process 2");
		assert!(
			wait_for_process_path(child1.id(), &test_helper, 2000)
				&& wait_for_process_path(child2.id(), &test_helper, 2000),
			"Test processes should start and be visible"
		);
		let processes = get_matching_processes(&log, &test_helper).unwrap();
		assert!(
			processes.len() >= 2,
			"Should have at least 2 matching test helper processes"
		);
		let result = wait_or_kill_for_test(&log, &test_helper);
		let _ = child1.wait();
		let _ = child2.wait();
		assert!(
			result.is_ok(),
			"Should succeed when killing multiple processes"
		);
	}

	#[test]
	fn test_wait_or_kill_ignores_same_name_at_different_path() {
		let _guard = PROCESS_TEST_MUTEX
			.lock()
			.unwrap_or_else(|err| err.into_inner());
		let log = setup_test_logger();
		let test_helper = get_test_helper_path();
		let temp_dir = tempfile::tempdir().expect("Failed to create temporary directory");
		let other_test_helper = temp_dir.path().join("test_helper.exe");
		std::fs::copy(&test_helper, &other_test_helper).expect("Failed to copy test helper");
		let mut other_child = start_test_process_at(&other_test_helper, &["run-forever"])
			.expect("Failed to start test process");
		assert!(
			wait_for_process_path(other_child.id(), &other_test_helper, 1000),
			"Test process should start and be visible"
		);

		let result = wait_or_kill_for_test(&log, &test_helper);
		let other_status = other_child
			.try_wait()
			.expect("Failed to query test process");
		let _ = other_child.kill();
		let _ = other_child.wait();

		assert!(
			result.is_ok(),
			"Unrelated same-name process should not block the update"
		);
		assert!(
			other_status.is_none(),
			"Unrelated same-name process should not be terminated"
		);
	}

	#[test]
	fn test_wait_or_kill_matches_process_after_executable_is_renamed() {
		let _guard = PROCESS_TEST_MUTEX
			.lock()
			.unwrap_or_else(|err| err.into_inner());
		let log = setup_test_logger();
		let test_helper = get_test_helper_path();
		let temp_dir = tempfile::tempdir().expect("Failed to create temporary directory");
		let current_path = temp_dir.path().join("test_helper.exe");
		let old_path = temp_dir.path().join("old_test_helper.exe");
		std::fs::copy(&test_helper, &current_path).expect("Failed to copy test helper");
		let mut child = start_test_process_at(&current_path, &["run-forever"])
			.expect("Failed to start process");
		assert!(
			wait_for_process_path(child.id(), &current_path, 1000),
			"Test process should start and be visible"
		);
		std::fs::rename(&current_path, &old_path).expect("Failed to rename running executable");

		let result = wait_or_kill_for_test(&log, &current_path);
		let child_status = child.try_wait().expect("Failed to query test process");
		if child_status.is_none() {
			let _ = child.kill();
			let _ = child.wait();
		}

		assert!(
			result.is_ok(),
			"Renaming the executable should not prevent terminating its process"
		);
		assert!(
			child_status.is_some(),
			"Process running from the renamed executable should be terminated"
		);
	}
}
