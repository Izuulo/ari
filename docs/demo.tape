# Hero GIF for the README. Reproduce with: cargo build --release && vhs docs/demo.tape
# At --speed 2.5 the replay reads: RETRYING ~5s, STUCK ~9s (panel.png), NEEDS YOU ~14s,
# ON TRACK ~18s, DONE ~24s. The demo slows down on its own while something is wrong.
Output docs/demo.gif

Set Shell "bash"
Set FontSize 15
Set Width 720
Set Height 410
Set Padding 14
Set Theme "TokyoNight"
Set TypingSpeed 0

Hide
Type "export PATH=$PWD/target/release:$PATH HOME=$(mktemp -d) PS1='$ ' && clear"
Enter
Type "ari demo --speed 2.5"
Enter
Sleep 300ms
Show

Sleep 11s
Screenshot docs/panel.png
Sleep 17s
