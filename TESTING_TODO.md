# Things to test by hand

These can't be checked off-screen and need someone at the PC. Tick each one off once it works,
and write down anything that looks wrong.

## Tray
- [ ] Settings > Start with Windows on, then sign out and back in (or restart): Purgeon is in the tray, not on screen, and opening it shows graphs that are already full
- [ ] Opened from the tray after a --tray start, the window comes back centred on the main screen
- [ ] Turning Start with Windows off removes it (Task Manager > Startup apps no longer lists Purgeon)
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

## FPS in the bar
- [ ] Without Elevate, the bar's first number reads "Elevate / for FPS"
- [ ] Click Elevate, approve, then open Roblox and press Alt+Shift+P: the big FPS number matches Roblox's own counter (Shift+F5) within a few frames (it used to read about a fifth low). It trails the game by about half a second
- [ ] In the FPS box, AVG, HIGH, LOW and 1% LOW fill in after a few seconds and start over each time the bar opens. 1% LOW drops when the game stutters
- [ ] With Claude, Discord or a browser in front, the box says "Not a game: it only redraws on changes" instead of a small, jumpy number
- [ ] Same with another game you have (a DirectX one); with the game capped at your monitor's refresh rate it should show that number
- [ ] Roblox launches and plays normally with Purgeon elevated and the bar open: no "third-party software" error, no kick
- [ ] Alt-tab to the desktop with the bar open: FPS shows "—" for Explorer, then the game's number again when you go back
- [ ] Task Manager: Purgeon's CPU stays about where it was before (well under 1%) while a game runs at high FPS
- [ ] A Vulkan or OpenGL game (or Roblox, whichever API it picks) also shows its FPS, matching its own counter

## Ping in the bar
- [ ] In a Roblox game (Purgeon elevated), PING shows "~20 ms"-ish within ~5 s and matches Roblox's NetworkPing (Shift+F5 performance stats), not the higher Data Ping in Shift+F3
- [ ] Alt-tab to a browser: PING switches to the site it talks to (or "—"); back to Roblox, it comes back
- [ ] Not elevated: PING shows "—" (the FPS box already says Elevate)
- [ ] Open Task Manager > Performance > Wi-Fi/Ethernet while in a game: Purgeon's own sending is tiny (one small ping a second)

## Sharp text
- [ ] Text in the window and the bar looks as crisp as other Windows apps next to it (no soft, smeared edges), and bold labels still look bold
- [ ] App icons (Dashboard tiles, Processes, Free up memory, the bar) look as sharp as the same icons in Task Manager, Brave and Antigravity included
- [ ] Text uses ClearType like the rest of Windows (very close up, letter edges have faint blue/orange tints, like in File Explorer). If it shows colour fringes on your screen, say so; turning ClearType off in Windows ("Adjust ClearType text") makes Purgeon use plain grey edges too

## Settings page
- [ ] Size: on Auto, the bar on the ultrawide is 125% (the Settings card says so). Is it easy to read at a glance now? Try 150% too
- [ ] Changing the size while the bar is open redraws it at the new size, and the text and the app icon stay sharp
- [ ] At 125% and 150%, the expand button still reacts to the mouse (it lights up on hover) and opens Purgeon with the morph
- [ ] Shortcut: click the Alt+Shift+P box, press Ctrl+Alt+O. It says "Saved", and Ctrl+Alt+O now opens the bar from a game. Alt+Shift+P no longer does
- [ ] While the box says "Press the new keys…", pressing Alt+Shift+P doesn't open the bar
- [ ] Pressing a single key, or Shift+a letter, is refused with a short reason; Esc or clicking elsewhere cancels
- [ ] Reset brings back Alt+Shift+P, and the tray menu shows the current shortcut
- [ ] The new shortcut still works after quitting and starting Purgeon again

## Boost a game (Dashboard)
- [ ] The row lists your installed games and no other apps; running ones come first
- [ ] On a PC with Epic, GOG, Riot, Ubisoft, EA, Battle.net or Xbox games: those show up too
- [ ] Open a game that runs on Java or .NET: its tile shows CPU and RAM, and other Java or .NET apps don't count
- [ ] Hover a tile in the Boost row: it lights up and says "Boost", and the tiles hold still
- [ ] Click "N more" and "Show less": the row unfolds and folds smoothly
- [ ] Click a game's tile. The row says "Priority: Above normal" and "Power plan: High performance"
- [ ] While the game runs, Task Manager > Details > right-click the game > Set priority shows Above normal
- [ ] Windows Settings > System > Power shows the High performance plan while the game runs (on some PCs only in Control Panel > Power Options)
- [ ] Close the game: the power plan goes back to the one you had before
- [ ] Start the game again: it gets boosted again on its own
- [ ] Click Stop boost while the game runs: priority back to Normal, power plan back
- [ ] Quit Purgeon from the tray while boosting: priority and power plan go back
- [ ] With Roblox (or another anti-cheat game), check what the card says. If it says the game doesn't allow it, that's expected
- [ ] Does the game feel any different? Honest answer welcome, it's meant as a small nudge
- [ ] As a standard user, the boosted card offers "Elevate to test". After approving, it offers "Run 5 min test"
- [ ] Run the test and keep playing in one spot: about 5 minutes later a notification appears, and the card shows deepest dip, 1% low and average FPS without and with Boost
- [ ] During the test with Steadier frames off, Task Manager shows the game's priority switching between Normal and Above normal; afterwards it is Above normal again
- [ ] Alt-tab out during the test: it says Paused and Boost is on; go back and that round runs again
- [ ] Cancel, Stop boost, closing the game and Quit during a test: none leaves Boost or the power plan in the wrong state
- [ ] Run the test twice in the same spot: both runs reach the same conclusion
- [ ] Steadier frames (experimental): switch it on with Roblox open. Does it say "preferring the 8 fast ones", or that Roblox refused?
- [ ] Click Lower on an app: Task Manager > Details shows Below normal. Switch to that app or play sound in it: Normal within a few seconds
- [ ] Undo, the switch, Stop boost, closing the game and Quit each put the lowered app back to Normal
- [ ] End Purgeon in Task Manager while an app is lowered, then start it again: the app is back at Normal
- [ ] Play normally (fights, getting shot or flung) through two or three tests with it on: do the deepest dips get shallower?
- [ ] Voice chat, music and the other apps you use still behave while it is on

## Free up memory (Dashboard)
- [ ] Leave Purgeon open for 10+ minutes with apps you're not using (Discord, Steam, a second browser). They show up in the list
- [ ] The app you're actively using never shows up there, and neither does a game that's running
- [ ] Close… on an app with a window: it closes like you clicked its X (it may ask to save)
- [ ] Close… on a tray-only app (Discord hidden in the tray): it ends, and the card says how much free memory changed
- [ ] Cancel in the confirm box really does nothing
- [ ] An app you haven't touched since before Purgeon started says "idle 16+ min" (at least that long), not a time that looks exact
- [ ] Quit and start Purgeon again right away: the idle times carry on instead of starting from zero

## Elevate (administrator mode)
- [ ] Clicking Elevate and approving UAC restarts Purgeon as administrator
- [ ] Disk page: the per-app disk speeds show up
- [ ] Network page: the per-app download and upload speeds show up
- [ ] Elevate works while Purgeon is in tray mode: only one tray icon afterwards, and the menu still works

## Look and feel
- [ ] Dashboard: CPU, Memory, Disk and GPU cards sit in one row at the top; the Disk card shows the busiest disk's active time with its read and write speeds
- [ ] Click a card: it gets an outline, its arrow turns down, and a panel opens below with a bigger graph and the numbers behind it. Click it again (or another card) to close or switch. "Open full page" goes to that page
- [ ] The scroll bar on Dashboard, CPU, Memory, Disk, Network, GPU, Sensors and Settings sits at the window's right edge
- [ ] The lighter content panel's rounded top-left corner has no thin line over its left edge
- [ ] Watch a graph's left end for a few updates: it slides along with the rest, with no gap flashing in before the new sample pushes it left
- [ ] Scroll any page down: cards slide straight under the title bar, with no dark band, fade or strip
- [ ] Dashboard: open the Memory card and scroll until its chart is off screen: the six boxes under it keep exactly the same grey
- [ ] Every scroll bar is just a grey thumb with no track, easy to see and to drag
- [ ] Hover around every page: no explanatory tooltips. Graphs still show value and time; a process's Memory number still shows its split
- [ ] Processes page: svchost, dwm, WmiPrvSE, MsMpEng and System show an icon instead of a letter (svchost, dwm and System get the plain Windows program icon, like in Task Manager)
- [ ] Processes page: each row shows the name with its PID underneath, then CPU, Memory, Disk (or I/O) and GPU with a thin bar under each number; rows doing nothing show no bars
- [ ] Processes page: click a row and a details panel slides in on the right (location, CPU, GPU, I/O, the memory split, video memory, threads, handles, End task). The × or Esc closes it; clicking the row again also closes it
- [ ] Processes page: End task in the details panel asks first, as before
- [ ] Dashboard: the "busy apps" card now sits under the four cards and shows the five apps holding the most memory as tiles, biggest first, all in GB (the bars are sized against the biggest). Hover a tile to see the exe name and how many processes it has
- [ ] Processes page: when the order changes, the row slide feels neither too fast nor too slow
- [ ] Shrink the window as small as it goes and drag its edges around with the pointer over the Dashboard graphs: no crash, and the GPU graph's readout puts the time on a second line

## Accuracy
- [ ] Open Roblox Studio: Purgeon's RAM number for it comes close to the ~10 GB that Resource Monitor showed

## Sensors page
- [ ] Scroll down: the CPU temperature tile reads "Needs a driver"
