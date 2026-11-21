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
- [ ] The bar is easy to read at a glance on the ultrawide. If the text feels too small, say so and it can get a size setting (125%, 150%) or scale up on wide screens by itself
- [ ] The System side's two rows read clearly: LOAD (how busy CPU, GPU, RAM are, plus download speed) and TEMP (how hot). CPU temperature shows "—" on purpose, since reading it needs a driver

## Boost an app (Overview page)
- [ ] Pick a game from "Choose an open app…" and click Boost. The card says "Priority raised" and "Power plan: High performance"
- [ ] While the game runs, Task Manager > Details > right-click the game > Set priority shows Above normal
- [ ] Windows Settings > System > Power shows the High performance plan while the game runs (on some PCs only in Control Panel > Power Options)
- [ ] Close the game: the power plan goes back to the one you had before
- [ ] Start the game again: it gets boosted again on its own
- [ ] Click Stop boost while the game runs: priority back to Normal, power plan back
- [ ] Quit Purgeon from the tray while boosting: priority and power plan go back
- [ ] With Roblox (or another anti-cheat game), check what the card says. If it says the game doesn't allow it, that's expected
- [ ] Does the game feel any different? Honest answer welcome, it's meant as a small nudge

## Free up memory (Overview page)
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
- [ ] Processes page: when the order changes, the row slide feels neither too fast nor too slow

## Accuracy
- [ ] Open Roblox Studio: Purgeon's RAM number for it comes close to the ~10 GB that Resource Monitor showed

## Sensors page
- [ ] Scroll down: the CPU temperature tile reads "Needs a driver"
