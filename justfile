name := "cosmic-ext-applet-claude-status"
app_id := "dev.techgeek1.CosmicExtAppletClaudeStatus"
prefix := "/usr"

build:
    cargo build --release

install: build
    install -Dm0755 target/release/{{name}} {{prefix}}/bin/{{name}}
    install -Dm0644 data/{{app_id}}.desktop {{prefix}}/share/applications/{{app_id}}.desktop
    install -Dm0644 data/icons/hicolor/scalable/apps/claude-status.png {{prefix}}/share/icons/hicolor/scalable/apps/{{app_id}}.png

uninstall:
    rm -f {{prefix}}/bin/{{name}}
    rm -f {{prefix}}/share/applications/{{app_id}}.desktop
    rm -f {{prefix}}/share/icons/hicolor/scalable/apps/{{app_id}}.png
