/*-----------------------------------------------------------------------------------------
 *  Copyright (c) Microsoft Corporation. All rights reserved.
 *  Licensed under the MIT License. See LICENSE in the project root for license information.
 *----------------------------------------------------------------------------------------*/

use std::sync::mpsc::Sender;
use std::{mem, ptr};
use strings::to_utf16;
use winapi::shared::basetsd::INT_PTR;
use winapi::shared::minwindef::{BOOL, DWORD, LPARAM, UINT, WPARAM};
use winapi::shared::ntdef::LPCWSTR;
use winapi::shared::windef::HWND;

extern "system" {
	pub fn ShutdownBlockReasonCreate(hWnd: HWND, pwszReason: LPCWSTR) -> BOOL;
	pub fn ShutdownBlockReasonDestroy(hWnd: HWND) -> BOOL;
}

struct DialogData {
	silent: bool,
	tx: Sender<ProgressWindow>,
}

unsafe extern "system" fn dlgproc(hwnd: HWND, msg: UINT, _: WPARAM, l: LPARAM) -> INT_PTR {
	use resources;
	use winapi::shared::windef::RECT;
	use winapi::um::commctrl::PBM_SETMARQUEE;
	use winapi::um::processthreadsapi::GetCurrentThreadId;
	use winapi::um::winuser::{
		GetDesktopWindow, GetWindowRect, SendDlgItemMessageW, SetWindowPos, ShowWindow,
		HWND_TOPMOST, SW_HIDE, WM_DESTROY, WM_INITDIALOG,
	};

	match msg {
		WM_INITDIALOG => {
			let data = &*(l as *const DialogData);
			if !data.silent {
				SendDlgItemMessageW(hwnd, resources::PROGRESS_SLIDER, PBM_SETMARQUEE, 1, 0);

				let mut rect: RECT = mem::uninitialized();
				GetWindowRect(hwnd, &mut rect);

				let width = rect.right - rect.left;
				let height = rect.bottom - rect.top;

				GetWindowRect(GetDesktopWindow(), &mut rect);

				SetWindowPos(
					hwnd,
					HWND_TOPMOST,
					rect.right / 2 - width / 2,
					rect.bottom / 2 - height / 2,
					width,
					height,
					0,
				);
			} else {
				ShowWindow(hwnd, SW_HIDE);
			}

			data.tx
				.send(ProgressWindow {
					ui_thread_id: GetCurrentThreadId(),
				})
				.unwrap();

			ShutdownBlockReasonCreate(hwnd, to_utf16("VS Code is updating...").as_ptr());
			0
		}
		WM_DESTROY => {
			ShutdownBlockReasonDestroy(hwnd);
			0
		}
		_ => 0,
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

pub fn run_progress_window(silent: bool, tx: Sender<ProgressWindow>) {
	use resources;
	use winapi::um::libloaderapi::GetModuleHandleW;
	use winapi::um::winuser::{DialogBoxParamW, MAKEINTRESOURCEW};

	let data = DialogData { silent, tx };

	unsafe {
		DialogBoxParamW(
			GetModuleHandleW(ptr::null_mut()),
			MAKEINTRESOURCEW(resources::PROGRESS_DIALOG),
			ptr::null_mut(),
			Some(dlgproc),
			(&data as *const DialogData) as LPARAM,
		);
	}
}

pub fn message_box(text: &str, caption: &str) -> i32 {
	use winapi::um::winuser::{MessageBoxW, MB_ICONERROR, MB_SYSTEMMODAL};

	unsafe {
		MessageBoxW(
			ptr::null_mut(),
			to_utf16(text).as_ptr(),
			to_utf16(caption).as_ptr(),
			MB_ICONERROR | MB_SYSTEMMODAL,
		)
	}
}
