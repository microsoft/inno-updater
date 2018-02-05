/*-----------------------------------------------------------------------------------------
 *  Copyright (c) Microsoft Corporation. All rights reserved.
 *  Licensed under the MIT License. See LICENSE in the project root for license information.
 *----------------------------------------------------------------------------------------*/

use std::{io, mem, ptr};
use std::path::PathBuf;
use winapi::shared::windef::HWND;
use winapi::shared::minwindef::{DWORD, LPARAM, LRESULT, UINT, WPARAM};
use winapi::um::libloaderapi::GetModuleHandleW;

fn utf16(value: &str) -> Vec<u16> {
	use std::ffi::OsStr;
	use std::os::windows::ffi::OsStrExt;
	use std::iter::once;

	OsStr::new(value).encode_wide().chain(once(0u16)).collect()
}

fn from_utf16(value: &[u16]) -> Result<String, io::Error> {
	use std::ffi::OsString;
	use std::os::windows::ffi::OsStringExt;

	let pos = value.iter().position(|&x| x == 0).unwrap_or(value.len());
	let value = &value[0..pos];

	OsString::from_wide(value)
		.into_string()
		.map_err(|_| io::Error::new(io::ErrorKind::Other, "Could convert from utf16"))
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: UINT, w: WPARAM, l: LPARAM) -> LRESULT {
	use winapi::um::winuser::{BeginPaint, DefWindowProcW, EndPaint, PostQuitMessage, PAINTSTRUCT,
	                          WM_DESTROY, WM_PAINT};
	use winapi::um::wingdi::{GetStockObject, SelectObject, SetBkMode, TextOutW, ANSI_VAR_FONT,
	                         TRANSPARENT};
	use winapi::ctypes::c_int;

	match msg {
		WM_PAINT => {
			let mut ps = PAINTSTRUCT {
				hdc: mem::uninitialized(),
				fErase: 0,
				rcPaint: mem::uninitialized(),
				fRestore: 0,
				fIncUpdate: 0,
				rgbReserved: [0; 32],
			};

			let hdc = BeginPaint(hwnd, &mut ps);
			SetBkMode(hdc, TRANSPARENT as c_int);

			let font = GetStockObject(ANSI_VAR_FONT as c_int);
			SelectObject(hdc, font);

			let text = utf16("Updating VS Code...");
			TextOutW(hdc, 15, 15, text.as_ptr(), text.len() as c_int);

			EndPaint(hwnd, &ps);

			0
		}
		WM_DESTROY => {
			PostQuitMessage(0);
			0
		}
		_ => DefWindowProcW(hwnd, msg, w, l),
	}
}

unsafe fn create_window_class(name: *const u16) {
	use winapi::um::winuser::{LoadCursorW, RegisterClassExW, COLOR_WINDOW, CS_HREDRAW, CS_VREDRAW,
	                          IDC_ARROW, WNDCLASSEXW};

	let class = WNDCLASSEXW {
		cbSize: mem::size_of::<WNDCLASSEXW>() as UINT,
		style: CS_HREDRAW | CS_VREDRAW,
		lpfnWndProc: Some(wndproc),
		cbClsExtra: 0,
		cbWndExtra: 0,
		hInstance: GetModuleHandleW(ptr::null_mut()),
		hIcon: ptr::null_mut(),
		hCursor: LoadCursorW(ptr::null_mut(), IDC_ARROW),
		hbrBackground: mem::transmute(COLOR_WINDOW as usize),
		lpszMenuName: ptr::null_mut(),
		lpszClassName: name,
		hIconSm: ptr::null_mut(),
	};

	let result = RegisterClassExW(&class);

	if result == 0 {
		panic!("Could not create window");
	}
}

pub struct ProgressWindow {
	ui_thread_id: DWORD,
}

unsafe impl Send for ProgressWindow {}

impl ProgressWindow {
	pub fn exit(&self) {
		use winapi::um::winuser::{PostThreadMessageW, WM_QUIT};

		unsafe {
			PostThreadMessageW(self.ui_thread_id, WM_QUIT, 0, 0);
		}
	}
}

pub fn create_progress_window() -> ProgressWindow {
	use winapi::shared::windef::RECT;
	use winapi::um::winuser::{CreateWindowExW, GetClientRect, GetDesktopWindow, GetWindowRect,
	                          SendMessageW, SetWindowPos, ShowWindow, UpdateWindow, CW_USEDEFAULT,
	                          HWND_TOPMOST, SW_SHOW, WS_CAPTION, WS_CHILD, WS_CLIPCHILDREN,
	                          WS_EX_COMPOSITED, WS_OVERLAPPED, WS_VISIBLE};
	use winapi::um::processthreadsapi::GetCurrentThreadId;
	use winapi::um::commctrl::{PBM_SETMARQUEE, PBS_MARQUEE, PROGRESS_CLASS};

	unsafe {
		let class_name = utf16("mainclass").as_ptr();
		create_window_class(class_name);

		let width = 280;
		let height = 90;

		let window = CreateWindowExW(
			WS_EX_COMPOSITED,
			class_name,
			utf16("VS Code").as_ptr(),
			WS_OVERLAPPED | WS_CAPTION | WS_CLIPCHILDREN,
			CW_USEDEFAULT,
			CW_USEDEFAULT,
			width,
			height,
			ptr::null_mut(),
			ptr::null_mut(),
			GetModuleHandleW(ptr::null()),
			ptr::null_mut(),
		);

		if window.is_null() {
			panic!("Could not create window");
		}

		ShowWindow(window, SW_SHOW);
		UpdateWindow(window);

		let mut rect: RECT = mem::uninitialized();
		GetClientRect(window, &mut rect);

		let width = width + width - rect.right;
		let height = height + height - rect.bottom;

		let desktop_window = GetDesktopWindow();
		GetWindowRect(desktop_window, &mut rect);

		SetWindowPos(
			window,
			HWND_TOPMOST,
			rect.right / 2 - width / 2,
			rect.bottom / 2 - height / 2,
			width,
			height,
			0,
		);

		let pbar = CreateWindowExW(
			0,
			utf16(PROGRESS_CLASS).as_ptr(),
			ptr::null(),
			WS_CHILD | WS_VISIBLE | PBS_MARQUEE,
			15,
			45,
			250,
			22,
			window,
			ptr::null_mut(),
			GetModuleHandleW(ptr::null()),
			ptr::null_mut(),
		);

		SendMessageW(pbar, PBM_SETMARQUEE, 1, 0);

		let ui_thread_id = GetCurrentThreadId();
		ProgressWindow { ui_thread_id }
	}
}

pub fn event_loop() {
	use winapi::um::winuser::{DispatchMessageW, GetMessageW, TranslateMessage, MSG};

	unsafe {
		let mut msg: MSG = mem::uninitialized();

		while GetMessageW(&mut msg, ptr::null_mut(), 0, 0) != 0 {
			TranslateMessage(&msg);
			DispatchMessageW(&msg);
		}
	}
}

pub fn message_box(text: &str, caption: &str) -> i32 {
	use winapi::um::winuser::{MessageBoxW, MB_ICONERROR};

	unsafe {
		MessageBoxW(
			ptr::null_mut(),
			utf16(text).as_ptr(),
			utf16(caption).as_ptr(),
			MB_ICONERROR,
		)
	}
}

pub struct RunningProcess {
	pub name: String,
	pub id: DWORD,
}

pub fn get_running_processes() -> Result<Vec<RunningProcess>, io::Error> {
	use winapi::shared::minwindef::TRUE;
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

pub fn get_process_path(process: &RunningProcess) -> Result<PathBuf, io::Error> {
	use winapi::shared::minwindef::MAX_PATH;
	use winapi::um::processthreadsapi::OpenProcess;
	use winapi::um::psapi::GetModuleFileNameExW;
	use winapi::um::handleapi::{CloseHandle, INVALID_HANDLE_VALUE};
	use winapi::um::winnt::{PROCESS_QUERY_INFORMATION, PROCESS_VM_READ};

	unsafe {
		let handle = OpenProcess(PROCESS_QUERY_INFORMATION | PROCESS_VM_READ, 0, process.id);

		if handle == INVALID_HANDLE_VALUE {
			return Err(io::Error::new(
				io::ErrorKind::Other,
				"could not open process",
			));
		}

		let mut raw_path = [0u16; MAX_PATH];
		let len = GetModuleFileNameExW(
			handle,
			ptr::null_mut(),
			raw_path.as_mut_ptr(),
			MAX_PATH as DWORD,
		) as usize;

		if len == 0 {
			return Err(io::Error::new(
				io::ErrorKind::Other,
				"could not get process file name",
			));
		}

		let path = from_utf16(&raw_path[0..len])?;
		CloseHandle(handle);

		Ok(PathBuf::from(path))
	}
}
