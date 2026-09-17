/*-----------------------------------------------------------------------------------------
 *  Copyright (c) Microsoft Corporation. All rights reserved.
 *  Licensed under the MIT License. See LICENSE in the project root for license information.
 *----------------------------------------------------------------------------------------*/

use std::ffi::OsString;
use std::os::windows::ffi::OsStringExt;
use std::path::{Path, PathBuf};
use std::{error, io, mem, thread, time};
use crate::strings::from_utf16;

pub struct RunningProcess {
	pub name: String,
	pub id: u32,
}

#[derive(Debug)]
struct ProcessHandle {
	handle: isize,
	#[cfg(test)]
	close_observer: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
}

impl ProcessHandle {
	fn open(process_id: u32, access: u32) -> io::Result<Self> {
		use windows_sys::Win32::System::Threading::OpenProcess;

		let handle = unsafe { OpenProcess(access, 0, process_id) };
		if handle == 0 {
			Err(io::Error::last_os_error())
		} else {
			Ok(Self {
				handle,
				#[cfg(test)]
				close_observer: None,
			})
		}
	}

	fn is_running(&self) -> io::Result<bool> {
		self.wait_for_exit(0).map(|exited| !exited)
	}

	fn wait_for_exit(&self, timeout_ms: u32) -> io::Result<bool> {
		use windows_sys::Win32::Foundation::{WAIT_FAILED, WAIT_OBJECT_0, WAIT_TIMEOUT};
		use windows_sys::Win32::System::Threading::WaitForSingleObject;

		let wait_result = unsafe { WaitForSingleObject(self.handle, timeout_ms) };
		if wait_result == WAIT_OBJECT_0 {
			Ok(true)
		} else if wait_result == WAIT_TIMEOUT {
			Ok(false)
		} else if wait_result == WAIT_FAILED {
			Err(io::Error::last_os_error())
		} else {
			Err(io::Error::other(format!(
				"Unexpected process wait result {}",
				wait_result
			)))
		}
	}
}

impl Drop for ProcessHandle {
	fn drop(&mut self) {
		use windows_sys::Win32::Foundation::CloseHandle;

		#[cfg(not(test))]
		unsafe {
			CloseHandle(self.handle);
		}
		#[cfg(test)]
		let closed = unsafe { CloseHandle(self.handle) } != 0;
		#[cfg(test)]
		if let Some(observer) = &self.close_observer {
			observer.store(closed, std::sync::atomic::Ordering::SeqCst);
		}
	}
}

#[derive(Debug)]
pub struct CapturedProcess {
	name: String,
	id: u32,
	wait_handle: ProcessHandle,
	termination_handle: Option<ProcessHandle>,
}

impl CapturedProcess {
	fn is_running(&self) -> io::Result<bool> {
		self.wait_handle.is_running()
	}

	fn terminate_and_wait(&self, log: &slog::Logger) -> Result<(), Box<dyn error::Error>> {
		use windows_sys::Win32::Foundation::{WAIT_FAILED, WAIT_OBJECT_0, WAIT_TIMEOUT};
		use windows_sys::Win32::System::Threading::{
			TerminateProcess, WaitForSingleObject,
		};

		if !self.is_running()? {
			info!(log, "{}, pid {} has already exited", self.name, self.id);
			return Ok(());
		}

		info!(
			log,
			"{}, pid {} is still running, attempting termination",
			self.name,
			self.id
		);

		let termination_handle = self.termination_handle.as_ref().ok_or_else(|| {
			io::Error::new(
				io::ErrorKind::PermissionDenied,
				format!("No termination handle is available for process {}", self.id),
			)
		})?;

		if unsafe { TerminateProcess(termination_handle.handle, 0) } == 0 {
			let err = io::Error::last_os_error();
			if !self.is_running()? {
				info!(log, "{}, pid {} exited before termination could be requested", self.name, self.id);
				return Ok(());
			}
			return Err(io::Error::new(
				err.kind(),
				format!("Failed to terminate process {}: {}", self.id, err),
			)
			.into());
		}

		info!(
			log,
			"Termination requested for {}, pid {}, waiting for exit",
			self.name,
			self.id
		);

		let wait_result = unsafe { WaitForSingleObject(termination_handle.handle, 30_000) };
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
			let err = io::Error::last_os_error();
			Err(io::Error::new(
				err.kind(),
				format!(
					"Failed waiting for process {} to exit: {}",
					self.id,
					err
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

const MAX_PROCESS_PATH_LENGTH: usize = 32_768;

fn get_process_path_with_query<F>(mut query: F) -> io::Result<PathBuf>
where
	F: FnMut(&mut [u16]) -> io::Result<usize>,
{
	use windows_sys::Win32::Foundation::{ERROR_INSUFFICIENT_BUFFER, MAX_PATH};

	let mut capacity = MAX_PATH as usize;
	loop {
		let mut raw_path = vec![0u16; capacity];
		match query(&mut raw_path) {
			Ok(len) => {
				raw_path.truncate(len);
				return Ok(PathBuf::from(OsString::from_wide(&raw_path)));
			}
			Err(err)
				if err.raw_os_error() == Some(ERROR_INSUFFICIENT_BUFFER as i32)
					&& capacity < MAX_PROCESS_PATH_LENGTH =>
			{
				capacity = (capacity * 2).min(MAX_PROCESS_PATH_LENGTH);
			}
			Err(err) => return Err(err),
		}
	}
}

fn get_process_path(handle: &ProcessHandle) -> io::Result<PathBuf> {
	use windows_sys::Win32::System::Threading::QueryFullProcessImageNameW;

	get_process_path_with_query(|raw_path| {
		let mut len = raw_path.len() as u32;
		if unsafe {
			QueryFullProcessImageNameW(handle.handle, 0, raw_path.as_mut_ptr(), &mut len)
		} == 0
		{
			Err(io::Error::last_os_error())
		} else {
			Ok(len as usize)
		}
	})
}

fn capture_termination_handle(
	log: &slog::Logger,
	process: &RunningProcess,
	wait_handle: &ProcessHandle,
) -> Option<ProcessHandle> {
	use windows_sys::Win32::System::Threading::{PROCESS_SYNCHRONIZE, PROCESS_TERMINATE};

	let termination_handle =
		match ProcessHandle::open(process.id, PROCESS_TERMINATE | PROCESS_SYNCHRONIZE) {
			Ok(handle) => handle,
			Err(err) => {
				match wait_handle.is_running() {
					Ok(false) => info!(
						log,
						"{}, pid {} exited before termination access could be captured",
						process.name,
						process.id
					),
					Ok(true) => warn!(
						log,
						"Verified target process {}, pid {} cannot be opened for termination it will only be monitored for natural exit: {}",
						process.name,
						process.id,
						err
					),
					Err(wait_err) => warn!(
						log,
						"Unable to capture termination access or check the state of verified target process {}, pid {}, it will only be monitored for natural exit. Open error: {}; state check error: {}",
						process.name,
						process.id,
						err,
						wait_err
					),
				}
				return None;
			}
		};

	match wait_handle.is_running() {
		Ok(true) => {
			info!(
				log,
				"Captured termination access for {}, pid {}",
				process.name,
				process.id
			);
			Some(termination_handle)
		}
		Ok(false) => {
			info!(
				log,
				"{}, pid {} exited while termination access was being captured",
				process.name,
				process.id
			);
			None
		}
		Err(err) => {
			warn!(
				log,
				"Unable to confirm that termination access still refers to the captured {}, pid {}; discarding termination access: {}",
				process.name,
				process.id,
				err
			);
			None
		}
	}
}

fn get_process_path_or_skip<F, W>(
	log: &slog::Logger,
	process: &RunningProcess,
	query_path: F,
	wait_for_exit: W,
) -> Option<PathBuf>
where
	F: FnOnce() -> io::Result<PathBuf>,
	W: FnOnce(u32) -> io::Result<bool>,
{
	use windows_sys::Win32::Foundation::ERROR_ACCESS_DENIED;

	let query_error = match query_path() {
		Ok(path) => return Some(path),
		Err(err) => err,
	};

	let exit_wait_ms = if query_error.raw_os_error() == Some(ERROR_ACCESS_DENIED as i32) {
		1_000
	} else {
		0
	};

	match wait_for_exit(exit_wait_ms) {
		Ok(true) => info!(
			log,
			"{}, pid {} exited before its path could be verified, path query error: {}",
			process.name,
			process.id,
			query_error
		),
		Ok(false) => warn!(
			log,
			"Unable to verify the path of process {}, pid {}, it will not be monitored or terminated after waiting {} ms for a possible exit: {}",
			process.name,
			process.id,
			exit_wait_ms,
			query_error
		),
		Err(wait_error) => warn!(
			log,
			"Unable to query or wait for process {}, pid {}, it will not be monitored or terminated. Path query error: {}; wait error: {}",
			process.name,
			process.id,
			query_error,
			wait_error
		),
	}

	None
}

fn capture_process(
	log: &slog::Logger,
	process: &RunningProcess,
	path: &Path,
) -> Option<CapturedProcess> {
	use windows_sys::Win32::Foundation::ERROR_INVALID_PARAMETER;
	use windows_sys::Win32::System::Threading::{
		PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE,
	};

	let handle = match ProcessHandle::open(
		process.id,
		PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE,
	) {
		Ok(handle) => handle,
		Err(err) if err.raw_os_error() == Some(ERROR_INVALID_PARAMETER as i32) => {
			info!(log, "{}, pid {} exited before it could be inspected", process.name, process.id);
			return None;
		}
		Err(err) => {
			warn!(
				log,
				"Unable to open process {}, pid {} for inspection, it will not be monitored or terminated: {}",
				process.name,
				process.id,
				err
			);
			return None;
		}
	};

	let process_path = get_process_path_or_skip(
		log,
		process,
		|| get_process_path(&handle),
		|timeout_ms| handle.wait_for_exit(timeout_ms),
	)?;

	info!(
		log,
		"Found {} running as pid {}", process_path.display(), process.id
	);

	if !paths_equal(&process_path, path) {
		info!(
			log,
			"Skipping pid {} because its path does not match the update target: {}",
			process.id,
			path.display()
		);
		return None;
	}

	info!(
		log,
		"Captured {}, pid {} for exit monitoring",
		process.name,
		process.id
	);
	let termination_handle = capture_termination_handle(log, process, &handle);
	match handle.is_running() {
		Ok(false) => {
			info!(log, "{}, pid {} exited while its handles were being captured", process.name, process.id);
			return None;
		}
		Ok(true) => {}
		Err(err) => warn!(
			log,
			"Unable to confirm the state of captured {}, pid {} it will remain monitored without making process shutdown fatal: {}",
			process.name,
			process.id,
			err
		),
	}

	Some(CapturedProcess {
		name: process.name.clone(),
		id: process.id,
		wait_handle: handle,
		termination_handle,
	})
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

	let candidates = get_running_processes()?
		.into_iter()
		.filter(|process| process.name.eq_ignore_ascii_case(file_name))
		.collect::<Vec<_>>();
	info!(
		log,
		"Found {} process candidates named {}, verifying their executable paths",
		candidates.len(),
		file_name
	);
	let target_processes = candidates
		.into_iter()
		.filter_map(|process| capture_process(log, &process, path))
		.collect::<Vec<_>>();

	if target_processes.is_empty() {
		info!(log, "No running {} processes matched the update target", file_name);
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
	wait_or_kill_with_grace(
		log,
		target_processes,
		60,
		time::Duration::from_millis(500),
	)
}

fn wait_or_kill_with_grace(
	log: &slog::Logger,
	target_processes: &[CapturedProcess],
	max_attempts: u32,
	poll_interval: time::Duration,
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
	let mut state_check_failures = Vec::new();

	loop {
		attempt += 1;

		info!(
			log,
			"Checking if {} processes are still running... (attempt {})", file_name, attempt
		);

		still_running = Vec::new();
		for process in target_processes {
			match process.is_running() {
				Ok(true) => still_running.push(process),
				Ok(false) => {}
				Err(err) => {
					if !state_check_failures.contains(&process.id) {
						warn!(
							log,
							"Unable to check whether {}, pid {} is still running, treating it as running for the remainder of the grace period: {}",
							process.name,
							process.id,
							err
						);
						state_check_failures.push(process.id);
					}
					still_running.push(process);
				}
			}
		}

		if still_running.is_empty() {
			info!(log, "All {} processes have exited", file_name);
			break;
		}

		if attempt >= max_attempts {
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
		thread::sleep(poll_interval);
	}

	if still_running.is_empty() {
		return Ok(());
	}

	info!(
		log,
		"Attempting best-effort termination of {} remaining {} processes: {:?}",
		still_running.len(),
		file_name,
		still_running.iter().map(|process| process.id).collect::<Vec<_>>()
	);

	let mut termination_failures = 0;
	for process in still_running {
		if let Err(err) = process.terminate_and_wait(log) {
			termination_failures += 1;
			warn!(
				log,
				"Unable to terminate {}, pid {}, continuing the update because process shutdown is best-effort: {}",
				process.name,
				process.id,
				err
			);
		}
	}

	if termination_failures == 0 {
		info!(log, "Best-effort process shutdown completed successfully");
	} else {
		warn!(
			log,
			"Best-effort process shutdown completed with {} unresolved process errors, the update will continue",
			termination_failures
		);
	}

	Ok(())
}

#[cfg(test)]
mod tests {
	use super::*;
	use std::io::Write;
	use std::path::PathBuf;
	use std::process::{Command, Child};
	use std::sync::Mutex;
	use std::thread;
	use std::time::Duration;
	use slog::{Logger, o, Drain};
	use slog_term::{PlainDecorator, FullFormat};

	struct TestWriter;

	impl Write for TestWriter {
		fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
			eprint!("{}", String::from_utf8_lossy(buffer));
			Ok(buffer.len())
		}

		fn flush(&mut self) -> io::Result<()> {
			Ok(())
		}
	}

	fn setup_test_logger() -> Logger {
		let decorator = PlainDecorator::new(TestWriter);
		let drain = FullFormat::new(decorator).build().fuse();
		let drain = Mutex::new(drain).fuse();
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

	struct TestChild {
		child: Child,
	}

	impl TestChild {
		fn spawn(path: &Path, args: &[&str]) -> Self {
			let child = Command::new(path)
				.args(args)
				.spawn()
				.expect("Failed to start test process");
			Self { child }
		}

		fn wait(&mut self) {
			self.child.wait().expect("Failed to wait for test process");
		}
	}

	impl Drop for TestChild {
		fn drop(&mut self) {
			match self.child.try_wait() {
				Ok(Some(_)) => {}
				Ok(None) => {
					if let Err(err) = self.child.kill() {
						eprintln!("Failed to terminate test process during cleanup: {}", err);
					}
					if let Err(err) = self.child.wait() {
						eprintln!("Failed to reap test process during cleanup: {}", err);
					}
				}
				Err(err) => {
					eprintln!("Failed to query test process during cleanup: {}", err);
				}
			}
		}
	}

	fn copy_test_helper(executable_name: &str) -> (tempfile::TempDir, PathBuf) {
		let directory = tempfile::tempdir().expect("Failed to create temporary directory");
		let path = directory.path().join(executable_name);
		std::fs::copy(get_test_helper_path(), &path).expect("Failed to copy test helper");
		(directory, path)
	}

	fn unique_test_executable(prefix: &str) -> String {
		use windows_sys::Win32::System::Threading::GetCurrentProcessId;

		format!("{}_{}.exe", prefix, unsafe { GetCurrentProcessId() })
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

	fn observe_handle_close(
		handle: &mut ProcessHandle,
	) -> std::sync::Arc<std::sync::atomic::AtomicBool> {
		let observer = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
		handle.close_observer = Some(observer.clone());
		observer
	}

	fn path_query_test_process() -> RunningProcess {
		RunningProcess {
			name: "path_query_test_helper.exe".to_string(),
			id: 42,
		}
	}

	#[test]
	fn test_path_query_access_denied_uses_exit_grace_period() {
		use windows_sys::Win32::Foundation::ERROR_ACCESS_DENIED;

		let log = setup_test_logger();
		let process = path_query_test_process();
		let mut observed_timeout = None;
		let result = get_process_path_or_skip(
			&log,
			&process,
			|| Err(io::Error::from_raw_os_error(ERROR_ACCESS_DENIED as i32)),
			|timeout_ms| {
				observed_timeout = Some(timeout_ms);
				Ok(true)
			},
		);

		assert!(result.is_none());
		assert_eq!(observed_timeout, Some(1_000));
	}

	#[test]
	fn test_path_query_access_denied_is_nonfatal_after_timeout() {
		use windows_sys::Win32::Foundation::ERROR_ACCESS_DENIED;

		let log = setup_test_logger();
		let process = path_query_test_process();
		let mut observed_timeout = None;
		let result = get_process_path_or_skip(
			&log,
			&process,
			|| Err(io::Error::from_raw_os_error(ERROR_ACCESS_DENIED as i32)),
			|timeout_ms| {
				observed_timeout = Some(timeout_ms);
				Ok(false)
			},
		);

		assert!(result.is_none());
		assert_eq!(observed_timeout, Some(1_000));
	}

	#[test]
	fn test_other_path_query_errors_are_nonfatal_without_delay() {
		use windows_sys::Win32::Foundation::ERROR_PARTIAL_COPY;

		let log = setup_test_logger();
		let process = path_query_test_process();
		let mut observed_timeout = None;
		let result = get_process_path_or_skip(
			&log,
			&process,
			|| Err(io::Error::from_raw_os_error(ERROR_PARTIAL_COPY as i32)),
			|timeout_ms| {
				observed_timeout = Some(timeout_ms);
				Ok(false)
			},
		);

		assert!(result.is_none());
		assert_eq!(observed_timeout, Some(0));
	}

	#[test]
	fn test_path_query_errors_skip_already_exited_process() {
		use windows_sys::Win32::Foundation::{ERROR_ACCESS_DENIED, ERROR_PARTIAL_COPY};

		let log = setup_test_logger();
		let process = path_query_test_process();
		for (error, expected_timeout) in [
			(ERROR_ACCESS_DENIED, 1_000),
			(ERROR_PARTIAL_COPY, 0),
		] {
			let mut observed_timeout = None;
			let result = get_process_path_or_skip(
				&log,
				&process,
				|| Err(io::Error::from_raw_os_error(error as i32)),
				|timeout_ms| {
					observed_timeout = Some(timeout_ms);
					Ok(true)
				},
			);
			assert!(result.is_none());
			assert_eq!(observed_timeout, Some(expected_timeout));
		}
	}

	#[test]
	fn test_path_query_wait_failures_are_nonfatal() {
		use windows_sys::Win32::Foundation::{ERROR_ACCESS_DENIED, ERROR_PARTIAL_COPY};

		let log = setup_test_logger();
		let process = path_query_test_process();
		let result = get_process_path_or_skip(
			&log,
			&process,
			|| Err(io::Error::from_raw_os_error(ERROR_PARTIAL_COPY as i32)),
			|timeout_ms| {
				assert_eq!(timeout_ms, 0);
				Err(io::Error::from_raw_os_error(ERROR_ACCESS_DENIED as i32))
			},
		);
		assert!(result.is_none());
	}

	#[test]
	fn test_process_handle_closes_on_drop() {
		use windows_sys::Win32::System::Threading::{
			GetCurrentProcessId, PROCESS_QUERY_LIMITED_INFORMATION,
		};

		let process_id = unsafe { GetCurrentProcessId() };
		let mut handle = ProcessHandle::open(process_id, PROCESS_QUERY_LIMITED_INFORMATION)
			.expect("Should open current process");
		let close_observer = observe_handle_close(&mut handle);
		drop(handle);
		assert!(close_observer.load(std::sync::atomic::Ordering::SeqCst));
	}

	#[test]
	fn test_process_path_query_retries_with_larger_buffer() {
		use std::os::windows::ffi::OsStrExt;
		use windows_sys::Win32::Foundation::ERROR_INSUFFICIENT_BUFFER;

		let expected_path = PathBuf::from(format!("C:\\{}", "a".repeat(300)));
		let encoded_path = expected_path.as_os_str().encode_wide().collect::<Vec<_>>();
		let mut capacities = Vec::new();

		let process_path = get_process_path_with_query(|buffer| {
			capacities.push(buffer.len());
			if buffer.len() < encoded_path.len() {
				return Err(io::Error::from_raw_os_error(
					ERROR_INSUFFICIENT_BUFFER as i32,
				));
			}

			buffer[..encoded_path.len()].copy_from_slice(&encoded_path);
			Ok(encoded_path.len())
		})
		.expect("Path query should succeed after growing the buffer");

		assert_eq!(process_path, expected_path);
		assert_eq!(capacities, vec![260, 520]);
	}

	#[test]
	fn test_process_path_query_stops_at_maximum_capacity() {
		use windows_sys::Win32::Foundation::ERROR_INSUFFICIENT_BUFFER;

		let mut capacities = Vec::new();
		let error = get_process_path_with_query(|buffer| {
			capacities.push(buffer.len());
			Err(io::Error::from_raw_os_error(
				ERROR_INSUFFICIENT_BUFFER as i32,
			))
		})
		.unwrap_err();

		assert_eq!(
			error.raw_os_error(),
			Some(ERROR_INSUFFICIENT_BUFFER as i32)
		);
		assert_eq!(capacities.first(), Some(&260));
		assert_eq!(capacities.last(), Some(&MAX_PROCESS_PATH_LENGTH));
		assert!(capacities.windows(2).all(|pair| pair[0] < pair[1]));
	}

	#[test]
	fn test_get_process_path_for_current_process() {
		use windows_sys::Win32::System::Threading::{
			GetCurrentProcessId, PROCESS_QUERY_LIMITED_INFORMATION,
		};

		let process_id = unsafe { GetCurrentProcessId() };
		let handle = ProcessHandle::open(process_id, PROCESS_QUERY_LIMITED_INFORMATION)
			.expect("Should open current process");
		let process_path = get_process_path(&handle).expect("Should query current process path");
		let expected_path = std::env::current_exe().expect("Should get current executable path");

		assert!(
			paths_equal(&process_path, &expected_path),
			"Queried path {:?} should match current executable {:?}",
			process_path,
			expected_path
		);
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
		let executable_name = unique_test_executable("natural_exit_helper");
		let (_directory, process_path) = copy_test_helper(&executable_name);
		let mut child = TestChild::spawn(&process_path, &["run-for-duration", "1"]);
		assert!(
			wait_for_process_start(&executable_name, 1000),
			"Natural-exit test process should start and be visible"
		);
		let processes = capture_running_processes(&log, &process_path).unwrap();
		assert_eq!(processes.len(), 1, "Should capture only the natural-exit test process");
		let result = wait_or_kill(&log, &processes);
		child.wait();
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
		let executable_name = unique_test_executable("multiple_process_helper");
		let (_directory, process_path) = copy_test_helper(&executable_name);
		let mut child1 = TestChild::spawn(&process_path, &["run-forever"]);
		let mut child2 = TestChild::spawn(&process_path, &["run-forever"]);
		assert!(
			wait_for_process_start(&executable_name, 2000),
			"Multiple-process test helpers should start and be visible"
		);
		let processes = get_running_processes().unwrap();
		let test_helper_count = processes
			.iter()
			.filter(|process| process.name == executable_name)
			.count();
		assert_eq!(test_helper_count, 2, "Should have exactly 2 isolated test processes");
		let processes = capture_running_processes(&log, &process_path).unwrap();
		assert_eq!(processes.len(), 2, "Should capture exactly the two isolated test processes");
		let result = wait_or_kill_with_grace(&log, &processes, 1, Duration::ZERO);
		child1.wait();
		child2.wait();
		assert!(result.is_ok(), "Should succeed when killing multiple processes");
	}

	#[test]
	fn test_wait_or_kill_process_after_executable_rename() {
		let log = setup_test_logger();
		let executable_name = unique_test_executable("rename_process_helper");
		let (directory, process_path) = copy_test_helper(&executable_name);
		let renamed_process_path = directory.path().join(format!("old_{}", executable_name));
		let mut child = TestChild::spawn(&process_path, &["run-forever"]);
		assert!(
			wait_for_process_start(&executable_name, 1000),
			"Rename test process should start and be visible"
		);
		let processes = capture_running_processes(&log, &process_path).unwrap();
		assert_eq!(processes.len(), 1, "Should capture the test process before renaming");

		std::fs::rename(&process_path, &renamed_process_path).expect("Failed to rename running test process");
		let result = wait_or_kill_with_grace(&log, &processes, 1, Duration::ZERO);
		child.wait();
		assert!(result.is_ok(), "Should kill and await the captured process after its executable is renamed");
	}
}
