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
| Per-process memory | Private memory only: misses memory the app shares | One "RAM" number that includes shared memory; click the process for the split and reserved memory | Caught 256 MB of shared memory that Task Manager showed as 0, matching the drop in available RAM. Private part exact |
| Per-process VRAM | Dedicated and shared GPU memory | Same counters, dedicated and shared kept apart, "—" when unavailable | Not tested (the bench stays off the GPU) |
| Per-process disk | Includes cached and network I/O | Same as Task Manager; as administrator, physical disk I/O only, from a kernel trace | Standard user: 256 of 256 MB, same as Task Manager. Administrator mode not tested yet |
| Per-process network | Not shown (Resource Monitor only) | As administrator: download and upload per program from the same kernel trace; open connections per program without admin | Not tested yet |
| Disk response time and queue | Per drive, across Task Manager and Resource Monitor | Per drive, all in one view | Same counters, no separate test |
| Own cost | Not measured here | About 33 MB of RAM with the window open (23 MB of it only Purgeon's), about 27 MB minimized, about 10 MB in the tray; the sampler uses 0.07% CPU and 13 ms per update | Measured |

## Tray and alerts

Closing the window doesn't quit Purgeon. It moves to the tray (the icons next to the clock) and
keeps an eye on your PC there, using about 10 MB of RAM and almost no CPU. Click its icon to
open it again. Right-click the icon to quit, or to turn the warnings off. Starting Purgeon again
while it's in the tray just opens the copy that's already running.

The graphs keep filling while Purgeon sits in the tray, so opening it later shows the last two
minutes straight away. To have that from the moment you sign in, turn on Settings > Start with
Windows: Purgeon then starts in the tray (from your own Run list, no administrator rights).

From the tray it shows a Windows notification only when something is wrong:

| Warning | When |
|---|---|
| Your PC is running out of memory | 92% or more of RAM in use for 20 seconds |
| An app is using a lot of memory | One app holds 30% of RAM (and at least 3 GB) for a minute |
| Your graphics card is running hot | The GPU is at 85°C or more for 30 seconds |

Each warning shows once, and only again after the problem has gone away and some time has passed
(20 minutes for memory, an hour per app, 30 minutes for the GPU). There's never more than one
warning every two minutes. Focus Assist / Do Not Disturb holds them back like any other
notification. The on/off choice is saved in `%APPDATA%\Purgeon\settings.ini`.

## Overlay

Press **Alt+Shift+P** anywhere to slide a slim bar down from the top of the screen, and press
it again to slide it away. It works from the tray too.

On the left it shows the app you're using (a game, Blender, a browser, anything): how much RAM,
VRAM, CPU and GPU it takes, its helper processes included. On the right, smaller, the whole PC:
in two rows: LOAD (CPU, GPU and RAM use, and download speed) and TEMP (how hot the CPU and GPU
are). They turn amber or red only when something's wrong. The button at the far right opens
Purgeon: the bar grows into the window, and the CPU, GPU, temperature and RAM numbers glide to
their places on the Dashboard while the rest of the page fades in (instant when Windows'
animation effects are off).

The bar is a normal always-on-top window that never takes focus. Nothing is added to or hooked
into the game, so it's safe with anti-cheat, but it can't show an FPS counter, and it can't
appear over exclusive fullscreen games (borderless and windowed are fine).

On the **Settings** page you can make the bar bigger (100%, 125%, 150%, or Auto, which uses
125% on 3440-wide screens and 150% on 4K ones) and pick another shortcut: click the shortcut and
press the new keys. It needs two of Ctrl, Alt and Shift (or one with an F key), so it never takes
a key other apps use on their own. Both are saved in `%APPDATA%\Purgeon\settings.ini`.

## Boost an app

On the Dashboard, pick a game or app and click **Boost**. Whenever it runs, Purgeon raises
its priority one step (Normal to Above normal, never higher) and switches Windows to the High
performance power plan. When the app closes, the boost is turned off, or Purgeon quits, the
priority and the power plan go back to what they were. Purgeon remembers the pick and boosts the
app every time it runs.

This is honest help, not magic: it makes a small difference when the CPU is busy with other
things, and none when the graphics card is what limits a game. Some games' anti-cheat refuses
the priority change; Purgeon then says so and leaves it at Normal. Laptops without a High
performance plan keep their plan.

## Free up memory

Also on the Dashboard. It lists apps that hold over 200 MB and that you haven't used for
10 minutes (never in front, near 0% CPU and GPU). Clicking **Close…** asks first. An app with a
window open is asked to close, the same as clicking its X, so it can ask to save. One with no
window (tray and background apps) is ended. A few seconds later the card reports how much free
memory actually changed. Purgeon never closes anything on its own, and there's no "clean all
memory" button: emptying every app's memory at once makes the PC slower, not faster.

## What is this, and why is it busy?

Click any program, in Processes, on the Dashboard, or in the "Who's using it" list at the top of
the CPU, Memory, Disk, Network and GPU pages, and a panel slides in on the right with:

- **What it is**, in plain words. For script runners it reads the command line, so `python.exe`
  says "Python script: `C:\bots\scraper.py`". Chrome, Edge and Electron apps (Discord, VS Code,
  Claude) name each process's job ("Tab or site", "Graphics helper", "Network service").
  `svchost.exe` lists the Windows services each copy hosts, and `msedgewebview2.exe` names the app
  it shows web pages for. Well-known Windows processes get a one-line description.
- **Why it's busy**: a few sentences built from fixed rules and the measured numbers, such as
  "Keeping one CPU thread fully busy", "Mostly writing to disk, 40 MB/s", "Using about 35% of
  the GPU's video decode engine: playing or streaming video", or "Memory grew by 300 MB in the
  last minute".
- **Last minute** graphs of its CPU, memory, disk, GPU and network.
- **Details** for a single process: when it started, who started it (with a link to the parent),
  its file, publisher, version, priority and full command line. Open connections list the
  addresses it talks to.
- Buttons to open the file's location, search the web for it, change its priority, or end it.

Processes always shows one row per program (chrome.exe ×30 is one row); its panel lists every
process, and clicking one opens that process. Per-program GPU and network numbers are Windows'
own estimates and are marked with ≈. A few protected Windows processes keep their command line
and file private even from an administrator.

## Keyboard

On the Processes page, start typing to filter, arrow keys move between programs (the panel
follows), Delete ends the selected program after a confirm, Esc clears the search or closes the
panel, Ctrl+F focuses search.
