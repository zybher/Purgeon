# Design: "Free memory from unused apps"

Status: design only. Nothing here is implemented.

## The problem

When RAM gets full, Windows trims working sets and pages memory out on its own, but late and
without telling anyone. People then reach for "RAM cleaner" tools, most of which empty every
working set at once. That makes things worse: the next time each program runs, it has to fault its
pages back in from disk.

Purgeon can do better because it already knows which apps are idle and how much memory each one
holds.

## What the feature does

On the Memory page, a card lists apps that hold a lot of memory but have done nothing for a while:

> **Apps you aren't using** · 3.1 GB could be freed
> Discord: 820 MB, idle for 42 min
> Steam: 610 MB, idle for 2 h
> [Free memory] for the selected apps

"Free memory" asks Windows to trim those apps' working sets. Their memory is moved to the
standby or modified list, and it comes back on its own if the app is used again. Nothing is
closed and no data is lost.

## Choosing candidates

An app (a group of processes with the same executable) is a candidate when all of these hold:

- Private working set above 200 MB (setting).
- CPU below 0.1% averaged over the last 10 minutes.
- No foreground window, and not the foreground app during the last 10 minutes.
- No audio playing (later: check the audio session API).
- Not on a built-in deny list: anything critical to Windows, anything with a game or full-screen
  window, Purgeon itself, and anything the user pinned with "Never suggest".
- Not running under another user, and not a protected process.

## How trimming works and what it needs

- Trimming another process uses `SetProcessWorkingSetSize(h, -1, -1)` or
  `EmptyWorkingSet`, which needs a handle with `PROCESS_SET_QUOTA` and
  `PROCESS_QUERY_LIMITED_INFORMATION`.
- That is more access than Purgeon asks for today. It is requested only when the user presses the
  button, for the selected processes only, and the handle is closed straight away.
- It never touches a process that has a game window, or any process in a deny list of anti-cheat
  protected executables. Anti-cheat drivers can treat outside handles with write-type rights as
  suspicious, so the deny list errs on the side of skipping.
- Most apps of the same user can be trimmed without administrator. Elevated or service processes
  are skipped unless Purgeon is elevated.

## Being honest about the effect

- Show "could be freed", not "will be freed": shared pages and pages the app touches again right
  away come back.
- After trimming, measure again after 5 seconds and report the real difference:
  "Freed 2.4 GB (Discord 700 MB, Steam 580 MB...)".
- Never offer a "free all memory" button. No scheduled automatic trimming in the first version.
- If available memory is above 25%, the card says so: "You have plenty of free memory; there is
  nothing to gain from freeing more."

## Open questions

1. Is idle-for-10-minutes the right default, or should it be per app?
2. Should standby list purging (admin only, `SystemMemoryListInformation` set) be offered? It
   discards the file cache and usually slows things down, so the proposal is no.
3. How to detect "playing audio" cheaply without loading extra system DLLs into Purgeon.
