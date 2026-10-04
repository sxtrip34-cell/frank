// Frank runs without a console window: Frank is the whole UI.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    frank_lib::run()
}
