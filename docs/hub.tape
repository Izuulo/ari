# Hub GIF and stills for the README. Reproduce with: cargo build --release && vhs docs/hub.tape
Output docs/hub.gif

Set Shell "bash"
Set FontSize 12
Set Width 1350
Set Height 700
Set Padding 12
Set Theme "TokyoNight"
Set TypingSpeed 45ms

Hide
Type "export PATH=$PWD/target/release:$PATH HOME=$(mktemp -d) PS1='$ ' && clear"
Enter
Show

# Speed 1: the two subagents run from 15.2s to 17.7s of the replay.
Type "ari demo --speed 1"
Sleep 400ms
Enter
Sleep 1s
Type "x"
Sleep 21500ms
Enter
Sleep 3s
Screenshot docs/hub-zoom.png
Escape
Sleep 800ms
Type "g"
Sleep 3s
Screenshot docs/hub-graph.png
Escape
Sleep 800ms
Type "q"
Sleep 300ms
