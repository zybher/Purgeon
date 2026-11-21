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
