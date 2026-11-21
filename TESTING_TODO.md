# Things to test by hand

These can't be checked off-screen and need someone at the PC. Tick each one off once it works,
and write down anything that looks wrong.

## Tray
- [ ] Closing the window sends Purgeon to the tray (the icons next to the clock) and doesn't quit it
- [ ] Clicking the tray icon opens the window again, in the same place and size
- [ ] Right-click menu: "Open Purgeon" works
- [ ] Right-click menu: "Warn me when something's wrong" turns on and off, and stays that way after a restart
- [ ] Right-click menu: "Quit Purgeon" closes it, and the icon goes away
- [ ] The first time you close it, a "Purgeon is still running" notification shows. Check that it reads well
- [ ] A warning notification looks right. To see one, open a lot of browser tabs until RAM is 92% full for about 20 seconds
- [ ] Starting Purgeon again while it's in the tray opens the running copy and doesn't start a second one
- [ ] The tray icon comes back after Explorer restarts (Task Manager > Windows Explorer > Restart)

## Overlay
- [ ] Alt+Shift+P slides the bar down from the top of the screen, and pressing it again slides it back up
- [ ] It shows the app you're using: try a game (borderless or windowed), Blender and a browser
- [ ] The game keeps focus while the bar is open: your keys and mouse still go to the game
- [ ] Clicking the expand button at the far right opens the full Purgeon window
- [ ] The tray menu's "Show overlay" item works
- [ ] With two monitors, the bar appears on the screen of the app you're using
- [ ] In exclusive fullscreen games the bar doesn't show (expected). Switching the game to borderless makes it show
- [ ] The RAM number for a game looks right next to the Processes page
- [ ] With Windows' animation effects turned off, the bar appears instantly, with no slide
- [ ] Click the expand button: the bar should grow into the full window, the CPU, GPU, temperature and RAM numbers glide to their spots on the Dashboard, and the page fades in. Try it with Purgeon maximized, not maximized, and closed to the tray
- [ ] During that, look for a single-frame flash of the old window before the bar starts growing. If you see one, say so
- [ ] With the bar on one screen and Purgeon on another, it should still look reasonable (the bar starts from the nearest edge of the window)
- [ ] With Windows' animation effects off, the expand button just opens the window, with no morph
- [ ] The System side's two rows read clearly: LOAD (how busy CPU, GPU, RAM are, plus download speed) and TEMP (how hot). CPU temperature shows "—" on purpose, since reading it needs a driver

## Settings page
- [ ] Size: on Auto, the bar on the ultrawide is 125% (the Settings card says so). Is it easy to read at a glance now? Try 150% too
- [ ] Changing the size while the bar is open redraws it at the new size, and the text stays sharp
- [ ] At 125% and 150%, the expand button still reacts to the mouse (it lights up on hover) and opens Purgeon with the morph
- [ ] Shortcut: click the Alt+Shift+P box, press Ctrl+Alt+O. It says "Saved", and Ctrl+Alt+O now opens the bar from a game. Alt+Shift+P no longer does
- [ ] While the box says "Press the new keys…", pressing Alt+Shift+P doesn't open the bar
- [ ] Pressing a single key, or Shift+a letter, is refused with a short reason; Esc or clicking elsewhere cancels
- [ ] Reset brings back Alt+Shift+P, and the tray menu shows the current shortcut
- [ ] The new shortcut still works after quitting and starting Purgeon again

## Boost an app (Dashboard)
- [ ] Pick a game from "Choose an open app…" and click Boost. The card says "Priority raised" and "Power plan: High performance"
- [ ] While the game runs, Task Manager > Details > right-click the game > Set priority shows Above normal
- [ ] Windows Settings > System > Power shows the High performance plan while the game runs (on some PCs only in Control Panel > Power Options)
- [ ] Close the game: the power plan goes back to the one you had before
- [ ] Start the game again: it gets boosted again on its own
- [ ] Click Stop boost while the game runs: priority back to Normal, power plan back
- [ ] Quit Purgeon from the tray while boosting: priority and power plan go back
- [ ] With Roblox (or another anti-cheat game), check what the card says. If it says the game doesn't allow it, that's expected
- [ ] Does the game feel any different? Honest answer welcome, it's meant as a small nudge

## Free up memory (Dashboard)
- [ ] Leave Purgeon open for 10+ minutes with apps you're not using (Discord, Steam, a second browser). They show up in the list
- [ ] The app you're actively using never shows up there, and neither does a game that's running
- [ ] Close… on an app with a window: it closes like you clicked its X (it may ask to save)
- [ ] Close… on a tray-only app (Discord hidden in the tray): it ends, and the card says how much free memory changed
- [ ] Cancel in the confirm box really does nothing

## Elevate (administrator mode)
- [ ] Clicking Elevate and approving UAC restarts Purgeon as administrator
- [ ] Disk page: the per-app disk speeds show up
- [ ] Network page: the per-app download and upload speeds show up
- [ ] Elevate works while Purgeon is in tray mode: only one tray icon afterwards, and the menu still works

## Look and feel
- [ ] Dashboard: the top card shows the five apps holding the most memory as tiles, biggest first, all in GB (the bars are sized against the biggest). Hover a tile to see the exe name and how many processes it has
- [ ] Hovering the Boost and Free up memory cards shows the longer explanation that used to be written out on the card
- [ ] Processes page: when the order changes, the row slide feels neither too fast nor too slow

## Accuracy
- [ ] Open Roblox Studio: Purgeon's RAM number for it comes close to the ~10 GB that Resource Monitor showed

## Sensors page
- [ ] Scroll down: the CPU temperature tile reads "Needs a driver"
