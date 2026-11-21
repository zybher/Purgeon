<img src="assets/purgeon-256.png" width="96" alt="Purgeon icon">

# Purgeon
A detailed Windows system monitor for CPU, RAM, VRAM, disk and more. Written fully in Rust.

Purgeon only reads system-wide counters and never touches another program's memory, so it is
safe to keep open next to games with anti-cheat. How each number is measured, and where it can be
wrong, is in [docs/ACCURACY.md](docs/ACCURACY.md).

## Compared with Task Manager

Measured on one machine (i7-14700F, Windows 11) with the bench
(`cargo run --release --features bench -- --bench`), which runs small loads of a known size and
checks what each tool saw. Full results are in [docs/ACCURACY.md](docs/ACCURACY.md#measured-results).
In short: **Purgeon matches Windows' own counters within about 2%, and on this machine it does
not beat them.** What it adds is detail in one place, not better numbers.

| | Task Manager | Purgeon | Measured |
|---|---|---|---|
| Per-process CPU | CPU time from Windows' scheduler | CPU cycles, counted at every context switch (Process Explorer's method) | Both within 2% of a known load, steady or bursty. A tie |
| Total CPU number | `% Processor Utility` (scaled by boost clock) | `% Processor Time` (share of time busy), with Task Manager's figure shown next to it | 0.5 points from `% Processor Time` on average |
| Per-process memory | Private working set | Private working set by default; switch to working set or commit | Exact: 512.0 of 512 MB, same as Task Manager |
| Per-process VRAM | Dedicated and shared GPU memory | Same counters, dedicated and shared kept apart, "—" when unavailable | Not tested (the bench stays off the GPU) |
| Per-process disk | Includes cached and network I/O | Same as Task Manager; as administrator, physical disk I/O only, from a kernel trace | Standard user: 256 of 256 MB, same as Task Manager. Administrator mode not tested yet |
| Per-process network | Not shown (Resource Monitor only) | As administrator: download and upload per program from the same kernel trace; open connections per program without admin | Not tested yet |
| Disk response time and queue | Per drive, across Task Manager and Resource Monitor | Per drive, all in one view | Same counters, no separate test |
| Own cost | Not measured here | About 51 MB of RAM with the window open; the sampler uses 0.07% CPU and 13 ms per update | Measured |

## Keyboard

On the Processes page: start typing to filter, arrow keys move the selection, Delete ends the
selected process after a confirm, Esc clears the search or selection, Ctrl+F focuses search.
