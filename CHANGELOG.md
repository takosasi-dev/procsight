# Changelog

All notable changes to this project are documented in this file.
The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

Planned as 0.1.0 after hands-on verification.

### Added

- Processes tab: tree and flat views, sorting, filtering, configurable columns, row highlighting, icons, detail panel (general, threads, DLLs, handles, signature, small graphs), copy to clipboard
- Process actions: end, end tree, end several, priority, CPU affinity, efficiency mode, suspend and resume (remembered across crashes), bring to front, open location, properties
- Touch-free processes: never opened by ProcSight; the number of opens per PID is shown in Diagnostics
- Find processes that lock a file (Restart Manager)
- Performance tab: CPU, memory, disks, network adapters and GPUs
- Startup tab (registry Run and startup folders) and Services tab
- Optional replacement of Task Manager for Ctrl+Shift+Esc, with `restore-taskmgr.reg`
- Settings, diagnostics, about window, tray icon, single instance, restart as administrator, dark and light themes
