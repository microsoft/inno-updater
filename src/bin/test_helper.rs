/*-----------------------------------------------------------------------------------------
 *  Copyright (c) Microsoft Corporation. All rights reserved.
 *  Licensed under the MIT License. See LICENSE in the project root for license information.
 *----------------------------------------------------------------------------------------*/

use std::env;
use std::thread;
use std::time::Duration;

fn main() {
    let args: Vec<String> = env::args().collect();
    
    if args.len() > 1 {
        match args[1].as_str() {
            "exit-immediately" => {
                std::process::exit(0);
            }
            "run-forever" => {
                loop {
                    thread::sleep(Duration::from_millis(100));
                }
            }
            "run-for-duration" => {
                if args.len() > 2 {
                    if let Ok(seconds) = args[2].parse::<u64>() {
                        thread::sleep(Duration::from_secs(seconds));
                    }
                }
                std::process::exit(0);
            }
            "crash" => {
                // Crash immediately for testing error handling
                panic!("Test crash");
            }
            _ => {
                eprintln!("Unknown command: {}", args[1]);
                std::process::exit(1);
            }
        }
    } else {
        // Default behavior: run for 1 second then exit
        thread::sleep(Duration::from_secs(1));
        std::process::exit(0);
    }
}