# Changelog

All notable changes to this project are documented in this file.
The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.4.0] - 2026-11-23

Pre-release. Eleven new settings; with the default settings ProcSight behaves as v0.3.0. The new behaviour is verified with windowless rendering tests only.

### Added

- Settings → 全般: tab to open at start (or the last tab), start in the normal view, the mini view, or hidden in the tray (only for a normal start; Ctrl+Shift+Esc and restarts as administrator always show the window)
- Settings → 見た目: six accent colors; where usage shading starts and where it is darkest
- Settings → 表とグラフ (new page): per-process CPU counted per core (100% = one logical processor; machine totals stay 0–100%, the `cpu>` filter uses the shown numbers), graph length 60, 120 or 300 samples (memory only), disk and network speeds in bits (bps, Kbps, Mbps, Gbps), double-click on a process row (details, open file location, or nothing), start the tree collapsed
- Settings → ミニ表示とトレイ (new page): which graphs and whether the top process appear in the mini view; tray icon meter for CPU, memory, or both
- README: resource usage compared with Task Manager

### Changed

- With the orange accent, the second graph line is blue so that the two lines stay distinguishable
- The tray tooltip is updated whenever its number changes, not only when the icon changes

## [0.3.0] - 2026-11-22

Pre-release. The new screens are verified with windowless rendering tests only; the mini view, always on top, the save dialog and toggling scheduled tasks need hands-on checks.

### Added

- Mini view (Ctrl+M, menu 表示): CPU, memory, GPU, disk and network in a small window; returns to the normal view when a confirmation is needed
- Always on top (menu 表示 and settings; off by default)
- Slower updates while a touch-free process runs (settings; off by default). Returns to the chosen interval 30 seconds after the last touch-free process exits
- "CPU の推移" column with the last 60 CPU samples per process (memory only)
- Save the visible process list as CSV (UTF-8 with BOM, CRLF, formula-like values neutralised)
- Startup tab: logon and boot tasks from Task Scheduler (enable and disable through the Task Scheduler API; tasks under \Microsoft\ are not listed) and Store app startup tasks (view only), with a location filter

### Fixed

- The window could shrink or grow on every start when the font size was not 100%

## [0.2.0] - 2026-11-22

Pre-release. The new screens are verified with windowless rendering tests only.

### Added

- Processes tab: usage shading for CPU, memory, I/O and GPU cells with machine totals in two-line column headers (setting "使用量で色を塗る")
- Filter syntax: `cpu>5`, `mem>1gb`, `gpu>10`, `io>1mb`, `pid:`, `user:`, `state:`; words are combined with AND. A "?" next to the filter box lists the syntax
- Row height setting (compact, normal, comfortable)
- Performance tab: CPU topology (sockets, cores, logical processors, L1 to L3 cache), memory breakdown (cached, paged and non-paged pool), top 5 processes per resource that jump to the Processes tab, larger stats, axis units and hover values on graphs
- Command palette (Ctrl+K) for tabs, actions, processes and services, and a keyboard shortcut list (F1)
- Status bar shows GPU usage, the number of running touch-free processes and the update interval

### Changed

- Redesigned tabs, detail panel, Startup and Services tables (status dots and badges, counts, full-width tables), settings, about window, file lock search and confirmation dialogs (dangerous buttons are red but "やめる" stays the default)
- System Idle Process always sorts to the bottom
- Filter words separated by spaces now must all match (previously the whole text was one substring)

## [0.1.0] - 2026-11-22

Pre-release. Administrator operations, replacing Task Manager and comparisons with Task Manager's numbers are not yet verified by hands-on use.

### Added

- Processes tab: tree and flat views, sorting, filtering, configurable columns, row highlighting, icons, detail panel (general, threads, DLLs, handles, signature, small graphs), copy to clipboard
- Process actions: end, end tree, end several, priority, CPU affinity, efficiency mode, suspend and resume (remembered across crashes), bring to front, open location, properties
- Touch-free processes: never opened by ProcSight; the number of opens per PID is shown in Diagnostics
- Find processes that lock a file (Restart Manager)
- Performance tab: CPU, memory, disks, network adapters and GPUs
- Startup tab (registry Run and startup folders) and Services tab
- Optional replacement of Task Manager for Ctrl+Shift+Esc, with `restore-taskmgr.reg`
- Settings, diagnostics, about window, tray icon, single instance, restart as administrator, dark and light themes

[0.3.0]: https://github.com/takosasi-dev/procsight/releases/tag/v0.3.0
[0.2.0]: https://github.com/takosasi-dev/procsight/releases/tag/v0.2.0
[0.1.0]: https://github.com/takosasi-dev/procsight/releases/tag/v0.1.0
