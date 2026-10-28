# Regenerate the TUI screenshots: cargo build --release && vhs docs/tui.tape
Output target/tui.gif
Set Shell zsh
Set Width 1400
Set Height 640
Set FontSize 15
Set Padding 10
Set Theme "Catppuccin Mocha"
Hide
Type "clear; ./target/release/ddreport tui --demo fixtures/sata_hdd_wd_red_usb.json fixtures/sata_ssd_samsung_870.json fixtures/nvme_apple.json fixtures/sata_hdd_seagate_failing.json"
Enter
Sleep 2s
Show
Screenshot docs/tui-overview.png
Down
Down
Down
Sleep 1s
Screenshot docs/tui-failing.png
Type "2"
Sleep 1s
Screenshot docs/tui-attributes.png
Sleep 1s
Hide
Type "q"
Sleep 500ms
