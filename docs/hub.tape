# Hub GIF and stills for the README. Reproduce with: cargo build --release && vhs docs/hub.tape
# At --speed 2 the replay reads: STUCK ~10s, NEEDS YOU ~15s, two Explore subagents ~21-24s,
# DONE ~27s. The demo slows down on its own while something is wrong.
# vhs Screenshot sometimes catches a half-painted screen; check both stills and rerun if torn.
Output docs/hub.gif

Set Shell "bash"
Set FontSize 12
Set Width 1350
Set Height 700
Set Padding 12
Set Theme "TokyoNight"
Set TypingSpeed 0

Hide
Type "export PATH=$PWD/target/release:$PATH HOME=$(mktemp -d) PS1='$ ' && clear"
Enter
Type "ari demo --speed 2"
Enter
Sleep 300ms
Type "x"
Sleep 200ms
Show

Sleep 29s
Enter
Sleep 3s
Screenshot docs/hub-zoom.png
Escape
Sleep 800ms
Type "g"
Sleep 3s
Screenshot docs/hub-graph.png
Escape
Sleep 2s
