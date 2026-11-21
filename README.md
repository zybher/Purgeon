<img src="assets/purgeon-256.png" width="96" alt="Purgeon icon">

# Purgeon
A detailed Windows system monitor for CPU, RAM, VRAM, disk and more. Written fully in Rust.

Purgeon only reads system-wide counters and never touches another program's memory, so it is
safe to keep open next to games with anti-cheat. How each number is measured, and where it can be
wrong, is in [docs/ACCURACY.md](docs/ACCURACY.md).

## Compared with Task Manager

Every row here is **pending verification**: it describes what Purgeon is built to do, checked on
one machine so far. The bench (`cargo run --release --features bench -- --bench`) will fill in
measured numbers.

| | Task Manager | Purgeon | Status |
|---|---|---|---|
| Per-process CPU | CPU time charged in 15.6 ms timer ticks | CPU cycles, counted at every context switch (Process Explorer's method) | pending verification |
| Total CPU number | `% Processor Utility` (scaled by boost clock) | `% Processor Time` (share of time busy), with Task Manager's figure shown next to it | pending verification |
| Per-process memory | Private working set | Private working set by default; switch to working set or commit | pending verification |
| Per-process VRAM | Dedicated and shared GPU memory | Same counters, dedicated and shared kept apart, "—" when unavailable | pending verification |
| Per-process disk | Includes cached and network I/O | As administrator: physical disk I/O only, from a kernel trace | pending verification |
| Per-process network | Not shown (Resource Monitor only) | As administrator: download and upload per program from the same kernel trace; open connections per program without admin | pending verification |
| Disk response time and queue | Per drive, across Task Manager and Resource Monitor | Per drive, all in one view | pending verification |
| Own memory use | About 120 MB | About 17 MB at idle on the test machine | pending verification |

## Keyboard

On the Processes page: start typing to filter, arrow keys move the selection, Delete ends the
selected process after a confirm, Esc clears the search or selection, Ctrl+F focuses search.
