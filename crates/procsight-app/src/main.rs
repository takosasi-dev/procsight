//! ProcSight.exe の入口。起動の手順は procsight_app::shell::run(DLL の探し方の固定が最初。INV-10)。
#![windows_subsystem = "windows"]

fn main() -> std::process::ExitCode {
    procsight_app::shell::run()
}
