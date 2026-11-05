# Hero GIF for the README. Reproduce with: cargo build --release && vhs docs/demo.tape
Output docs/demo.gif

Set Shell "bash"
Set FontSize 15
Set Width 720
Set Height 470
Set Padding 14
Set Theme "TokyoNight"
Set TypingSpeed 45ms

Hide
Type "export PATH=$PWD/target/release:$PATH PS1='$ ' && clear"
Enter
Show

Type "ari demo --speed 1.6"
Sleep 400ms
Enter
Sleep 19s
Type "q"
Sleep 300ms
