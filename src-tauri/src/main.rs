// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    if std::env::args().any(|argument| argument == "--version" || argument == "-V") {
        println!("ServerDesk {}", env!("CARGO_PKG_VERSION"));
        return;
    }

    if std::env::args().any(|argument| argument == "--purge-data") {
        std::process::exit(serverdesk_lib::purge_local_data_cli());
    }

    serverdesk_lib::run()
}
