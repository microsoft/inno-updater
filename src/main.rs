extern crate byteorder;
extern crate crc;
#[macro_use] extern crate native_windows_gui as nwg;
extern crate user32;
use nwg::{Event, Ui, simple_message, fatal_message, dispatch_events};

mod blockio;
mod strings;
mod model;

use std::fs;
use std::io;
use std::thread;
use std::time;
use std::io::prelude::*;
use std::vec::Vec;
use std::collections::HashMap;

// MAIN

fn print_statistics(recs: &[model::FileRec]) {
	let mut map: HashMap<u16, u32> = HashMap::new();

	for rec in recs {
		let count = map.entry(rec.typ as u16).or_insert(0);
		*count += 1;
	}

	println!("Statistics");

	for (k, c) in &map {
		println!("records 0x{:x} {}", k, c);
	}
}

fn patch() {
	let input_file = fs::File::open("unins000.dat").expect("file not found");
	let mut input = io::BufReader::new(input_file);

	let header = model::Header::from_reader(&mut input);
	let mut reader = blockio::BlockRead::new(&mut input);
	let mut recs = Vec::with_capacity(header.num_recs);

	for _ in 0..header.num_recs {
		let mut rec = model::FileRec::from_reader(&mut reader);

		match rec.typ {
			model::UninstallRecTyp::DeleteDirOrFiles | model::UninstallRecTyp::DeleteFile => rec.rebase(
				"C:\\Program Files (x86)\\ProcMon\\update",
				"C:\\Program Files (x86)\\ProcMon",
			),
			_ => (),
		}

		recs.push(rec);
	}

	let output_file = fs::File::create("output.dat").expect("could not create file");
	let mut output = io::BufWriter::new(output_file);

	header.to_writer(&mut output);

	let mut writer = blockio::BlockWrite::new(&mut output);

	for rec in recs {
		rec.to_writer(&mut writer);
	}

	writer.flush().expect("flush");
	// println!("{:?}", header);
 // print_statistics(&recs);

}

#[derive(Debug, Clone, Hash)]
pub enum AppId {
    // Controls
    MainWindow,
    // NameInput,
		ProgressBar,
    HelloButton,
    Label(u8),   // Ids for static controls that won't be referenced in the Ui logic can be shortened this way.

    // Events
    SayHello,

    // Resources
    MainFont,
    TextFont
}

use AppId::*; // Shortcut

pub fn setup_ui(ui: &Ui<AppId>) -> Result<(), nwg::Error> {

    // nwg_font!(family="Arial"; size=27)
    let f1 = nwg_font!(family="Segoe UI"; size=27);

    // nwg_font!(family="Arial"; size=17)
    let f2 = nwg::FontT {
        family: "Segoe UI", size: 17,
        weight: nwg::constants::FONT_WEIGHT_NORMAL,
        decoration: nwg::constants::FONT_DECO_NORMAL,
    };

    // nwg_window!( title="Template Example"; size=(280, 105))
    let window = nwg::WindowT {
        title: "No template",
        position: (100, 100), size: (280, 105),
        resizable: false, visible: true, disabled: false,
        exit_on_close: true
    };

    // nwg_label!( parent="MainWindow"; [...] font=Some("TextFont") )
    let label = nwg::LabelT {
        text: "Your Name: ",
        position: (5,15), size: (80, 25),
        visible: true, disabled: false,
        align: nwg::constants::HTextAlign::Left,
        parent: MainWindow, font: Some(TextFont)
    };

    // nwg_textinput!( parent="MainWindow"; [..] font=Some("TextFont") )
    // let tedit = nwg::TextInputT::<_, &'static str, _> {
    //     text: "",
    //     position: (85,13), size: (185,22),
    //     visible: true, disabled: false, readonly: false, password: false,
    //     limit: 32_767, placeholder: None,
    //     parent: MainWindow, font: Some(TextFont)
    // };

		let pbar = nwg_progressbar!(parent=MainWindow; position=(85,13); size=(185,22));

    // nwg_button!( parent="MainWindow"; [..] font=Some("MainFont") )
    let hellbtn = nwg::ButtonT {
        text: "Hello World!",
        position: (5, 45), size: (270, 50),
        visible: true, disabled: false,
        parent: MainWindow, font: Some(MainFont)
    };

    // resources:
    ui.pack_resource(&MainFont, f1);
    ui.pack_resource(&TextFont, f2);

    // controls:
    ui.pack_control(&MainWindow, window);
    ui.pack_control(&Label(0), label);
    // ui.pack_control(&NameInput, tedit);
    ui.pack_control(&ProgressBar, pbar);
    ui.pack_control(&HelloButton, hellbtn);

    // events:
    ui.bind(&HelloButton, &SayHello, Event::Click, |ui,_,_,_| {
        if let Ok(pbar) = ui.get::<nwg::ProgressBar>(&ProgressBar) {
            pbar.set_value(pbar.get_value()+10);
        } else {
            panic!()
        }
    });

    ui.commit()
}

fn show_window() {
    let app: Ui<AppId>;

    match Ui::new() {
        Ok(_app) => { app = _app; },
        Err(e) => { fatal_message("Fatal Error", &format!("{:?}", e) ); }
    }

    if let Err(e) = setup_ui(&app) {
        fatal_message("Fatal Error", &format!("{:?}", e));
    }

		let pbarHandle = app.handle_of(&ProgressBar).expect("pbar handle");

		thread::spawn(|| {
			// thread::sleep(time::Duration::from_millis(1000));
			// appRef.has_id(&ProgressBar);
			// let pbar = app.get::<nwg::ProgressBar>(&ProgressBar).expect("pbar");

			for i in 0..100 {
				// pbar.set_value(i);
				user32::PostMessageW(pbarHandle, PBM_SETPOS, i, 0);
				thread::sleep(time::Duration::from_millis(100));
			}
    });

    dispatch_events();
}

fn main() {
	show_window();
}
