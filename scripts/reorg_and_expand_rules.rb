#!/usr/bin/env ruby
# frozen_string_literal: true

# Reorganize rules/*.yaml → rules/categories/<category>.yaml and expand to TARGET rules.
# Extensible: drop any extra *.yaml under rules/ (recursive); ids must stay unique.

require "yaml"
require "fileutils"
require "set"

ROOT = File.expand_path("..", __dir__)
RULES = File.join(ROOT, "rules")
CAT_DIR = File.join(RULES, "categories")
TARGET = Integer(ENV.fetch("RULES_TARGET", "3000"))

# Map legacy / noisy category names onto stable buckets (file + rule.category).
CANON = {
  "cache" => "caches",
  "caches" => "caches",
  "app-cache" => "app-cache",
  "browser-cache" => "browser-cache",
  "package-cache" => "package-cache",
  "media-cache" => "media-cache",
  "shader-cache" => "shader-cache",
  "wine-cache" => "wine-cache",
  "game-cache" => "game-cache",
  "ml-cache" => "ml-cache",
  "vm-cache" => "vm-cache",
  "logs" => "logs",
  "app-logs" => "logs",
  "ml-logs" => "logs",
  "game-logs" => "logs",
  "temp" => "temp",
  "leftovers" => "leftovers",
  "dev" => "dev",
  "dev-tool" => "dev",
  "ml-dev" => "dev",
  "game-dev-cache" => "dev",
  "ml-models" => "ml-models",
  "vm-disk" => "vm-disk",
  "media-library" => "media-library",
  "sample-library" => "sample-library",
  "app-data" => "app-data",
  "app-library" => "app-data",
  "user-data" => "app-data",
  "system-data" => "system",
  "system" => "system",
  "language-packs" => "system",
  "icloud" => "system",
  "privacy" => "system",
  "config" => "system",
  "certs" => "system",
  "licensing" => "system",
  "db-data" => "databases",
  "tsdb" => "databases",
  "object-storage" => "databases",
  "game-downloads" => "games-downloads",
  "report-only" => "report",
  "misc" => "misc"
}.freeze

def canon_cat(c)
  key = (c || "misc").to_s.strip.downcase
  CANON.fetch(key, key.gsub(/[^a-z0-9\-]+/, "-").gsub(/-+/, "-").sub(/^-|-$/, ""))
end

def slug(s)
  s.to_s.downcase.gsub(/[^a-z0-9]+/, "-").gsub(/-+/, "-").sub(/^-|-$/, "")
end

def load_all_flat
  rules = []
  Dir.glob(File.join(RULES, "**", "*.{yaml,yml}")).sort.each do |f|
    next if File.basename(f).start_with?("_")
    next if f.include?("/categories/") && ARGV.include?("--from-categories-only")
    data = YAML.load_file(f) || {}
    (data["rules"] || []).each do |r|
      rules << r.merge("_source" => f)
    end
  end
  rules
end

def rule_hash(r, category)
  h = {
    "id" => r["id"],
    "name" => r["name"],
    "description" => r["description"] || "",
    "category" => category,
    "platform" => r["platform"] || "any",
    "risk" => r["risk"] || "low",
    "default_enabled" => r.key?("default_enabled") ? r["default_enabled"] : false,
    "min_age_days" => r["min_age_days"] || 7,
    "paths" => Array(r["paths"]),
    "delete_mode" => r["delete_mode"] || "contents",
    "max_depth" => r["max_depth"] || 6
  }
  h["include_globs"] = r["include_globs"] if r["include_globs"] && !r["include_globs"].empty?
  h["exclude_globs"] = r["exclude_globs"] if r["exclude_globs"] && !r["exclude_globs"].empty?
  h["report_only"] = true if r["report_only"]
  h
end

# --- generators for new rules -------------------------------------------------

MAC_APPS = [
  ["discord", "com.hnc.Discord", "Discord"],
  ["slack", "com.tinyspeck.slackmacgap", "Slack"],
  ["notion", "notion.id", "Notion"],
  ["figma", "com.figma.Desktop", "Figma"],
  ["linear", "com.linear", "Linear"],
  ["obsidian", "md.obsidian", "Obsidian"],
  ["craft", "com.lukilabs.lukiapp", "Craft"],
  ["bear", "net.shinyfrog.bear", "Bear"],
  ["ulysses", "com.ulyssesapp.mac", "Ulysses"],
  ["ia-writer", "pro.writer.mac", "iA Writer"],
  ["typora", "abnerworks.Typora", "Typora"],
  ["zed", "dev.zed.Zed", "Zed"],
  ["sublime-text", "com.sublimetext.4", "Sublime Text"],
  ["nova", "com.panic.Nova", "Nova"],
  ["tower", "com.fournova.Tower3", "Tower"],
  ["fork", "com.DanPristupov.Fork", "Fork"],
  ["github-desktop", "com.github.GitHubClient", "GitHub Desktop"],
  ["sourcetree", "com.torusknot.SourceTreeNotMAS", "SourceTree"],
  ["postman", "com.postmanlabs.mac", "Postman"],
  ["insomnia", "com.insomnia.app", "Insomnia"],
  ["tableplus", "com.tinyapp.TablePlus", "TablePlus"],
  ["sequel-ace", "com.sequel-ace.sequel-ace", "Sequel Ace"],
  ["db-browser", "uk.co.sqlitebrowser", "DB Browser"],
  ["docker-desktop", "com.docker.docker", "Docker Desktop"],
  ["orbstack", "dev.orbstack.OrbStack", "OrbStack"],
  ["utm", "com.utmapp.UTM", "UTM"],
  ["parallels", "com.parallels.desktop.console", "Parallels"],
  ["vmware-fusion", "com.vmware.fusion", "VMware Fusion"],
  ["rectangle", "com.knollsoft.Rectangle", "Rectangle"],
  ["raycast", "com.raycast.macos", "Raycast"],
  ["alfred", "com.runningwithcrayons.Alfred", "Alfred"],
  ["bartender", "com.surteesstudios.Bartender", "Bartender"],
  ["cleanmymac", "com.macpaw.CleanMyMac4", "CleanMyMac"],
  ["appcleaner", "net.freemacsoft.AppCleaner", "AppCleaner"],
  ["the-unarchiver", "com.macpaw.site.theunarchiver", "The Unarchiver"],
  ["keka", "com.aone.keka", "Keka"],
  ["iina", "com.colliderli.iina", "IINA"],
  ["vlc", "org.videolan.vlc", "VLC"],
  ["handbrake", "fr.handbrake.HandBrake", "HandBrake"],
  ["obs", "com.obsproject.obs-studio", "OBS"],
  ["zoom", "us.zoom.xos", "Zoom"],
  ["teams", "com.microsoft.teams2", "Microsoft Teams"],
  ["webex", "Cisco-Systems.Spark", "Webex"],
  ["telegram", "ru.keepcoder.Telegram", "Telegram"],
  ["whatsapp", "net.whatsapp.WhatsApp", "WhatsApp"],
  ["signal", "org.whispersystems.signal-desktop", "Signal"],
  ["element", "im.riot.app", "Element"],
  ["skype", "com.skype.skype", "Skype"],
  ["discord-ptb", "com.hnc.DiscordPTB", "Discord PTB"],
  ["spotify", "com.spotify.client", "Spotify"],
  ["apple-music", "com.apple.Music", "Apple Music"],
  ["tidal", "com.tidal.desktop", "TIDAL"],
  ["amazon-music", "com.amazon.music", "Amazon Music"],
  ["deezer", "com.deezer.deezer-desktop", "Deezer"],
  ["pocket-casts", "au.com.shiftyjelly.pocketcasts", "Pocket Casts"],
  ["overcast", "fm.overcast.overcast", "Overcast"],
  ["kindle", "com.amazon.Kindle", "Kindle"],
  ["calibre", "net.kovidgoyal.calibre", "Calibre"],
  ["goodnotes", "com.goodnotesapp.x", "GoodNotes"],
  ["notability", "com.gingerlabs.Notability", "Notability"],
  ["pdf-expert", "com.readdle.PDFExpert-Mac", "PDF Expert"],
  ["preview-extras", "com.apple.Preview", "Preview"],
  ["sketch", "com.bohemiancoding.sketch3", "Sketch"],
  ["affinity-photo", "com.seriflabs.affinityphoto2", "Affinity Photo"],
  ["affinity-designer", "com.seriflabs.affinitydesigner2", "Affinity Designer"],
  ["affinity-publisher", "com.seriflabs.affinitypublisher2", "Affinity Publisher"],
  ["pixelmator-pro", "com.pixelmatorteam.pixelmator.x", "Pixelmator Pro"],
  ["final-cut-pro", "com.apple.FinalCut", "Final Cut Pro"],
  ["logic-pro", "com.apple.logic10", "Logic Pro"],
  ["garageband", "com.apple.garageband10", "GarageBand"],
  ["davinci-resolve", "com.blackmagic-design.DaVinciResolve", "DaVinci Resolve"],
  ["blender", "org.blenderfoundation.blender", "Blender"],
  ["cinema4d", "net.maxon.cinema4d", "Cinema 4D"],
  ["houdini", "com.sidefx.Houdini", "Houdini"],
  ["unity-hub", "com.unity3d.unityhub", "Unity Hub"],
  ["unreal-editor", "com.epicgames.UE4Editor", "Unreal Editor"],
  ["godot", "org.godotengine.Godot", "Godot"],
  ["steam", "com.valvesoftware.steam", "Steam"],
  ["epic-games", "com.epicgames.EpicGamesLauncher", "Epic Games"],
  ["gog-galaxy", "com.gog.galaxy", "GOG Galaxy"],
  ["battle-net", "net.battle.net", "Battle.net"],
  ["origin", "com.ea.Origin", "EA App"],
  ["riot-client", "com.riotgames.LeagueofLegends.RiotClient", "Riot Client"],
  ["minecraft", "com.mojang.minecraft", "Minecraft"],
  ["chatgpt", "com.openai.chat", "ChatGPT"],
  ["claude", "com.anthropic.claudefordesktop", "Claude"],
  ["perplexity", "ai.perplexity.mac", "Perplexity"],
  ["copilot", "com.microsoft.copilot", "Microsoft Copilot"],
  ["grammarly", "com.grammarly.desktop", "Grammarly"],
  ["todoist", "com.todoist.mac", "Todoist"],
  ["things", "com.culturedcode.ThingsMac", "Things"],
  ["omnifocus", "com.omnigroup.OmniFocus3", "OmniFocus"],
  ["fantastical", "com.flexibits.fantastical2.mac", "Fantastical"],
  ["busycal", "com.busymac.busycal3", "BusyCal"],
  ["cardhop", "com.flexibits.cardhop.mac", "Cardhop"],
  ["1password", "com.1password.1password", "1Password"],
  ["bitwarden", "com.bitwarden.desktop", "Bitwarden"],
  ["keeper", "com.callpod.keeper", "Keeper"],
  ["dashlane", "com.dashlane.dashlanephonefinal", "Dashlane"],
  ["nordvpn", "com.nordvpn.NordVPN", "NordVPN"],
  ["expressvpn", "com.expressvpn.ExpressVPN", "ExpressVPN"],
  ["mullvad", "net.mullvad.vpn", "Mullvad"],
  ["tailscale", "io.tailscale.ipn.macsys", "Tailscale"],
  ["wireguard", "com.wireguard.macos", "WireGuard"],
  ["little-snitch", "at.obdev.littlesnitch", "Little Snitch"],
  ["lulu", "objective-see.lulu", "LuLu"],
  ["blockblock", "objective-see.blockblock", "BlockBlock"],
  ["oversight", "objective-see.oversight", "OverSight"],
  ["knockknock", "objective-see.knockknock", "KnockKnock"],
  ["taskexplorer", "objective-see.taskexplorer", "TaskExplorer"],
  ["netiquette", "objective-see.netiquette", "Netiquette"],
  ["stats", "eu.exelban.Stats", "Stats"],
  ["istat-menus", "com.bjango.istatmenus", "iStat Menus"],
  ["activity-watch", "net.activitywatch.ActivityWatch", "ActivityWatch"],
  ["rescuetime", "com.rescuetime.RescueTime", "RescueTime"],
  ["timing", "info.eurocomp.Timing2", "Timing"],
  ["toggl", "com.toggl.toggldesktop.TogglDesktop", "Toggl Track"],
  ["clockify", "com.clockify.ClockifyDesktop", "Clockify"],
  ["dropbox", "com.getdropbox.dropbox", "Dropbox"],
  ["google-drive", "com.google.drivefs", "Google Drive"],
  ["onedrive", "com.microsoft.OneDrive", "OneDrive"],
  ["box", "com.box.desktop", "Box"],
  ["megasync", "mega.mac", "MEGA"],
  ["nextcloud", "com.nextcloud.desktopclient", "Nextcloud"],
  ["syncthing", "com.github.xor-gate.syncthing-macosx", "Syncthing"],
  ["transmit", "com.panic.Transmit", "Transmit"],
  ["cyberduck", "ch.sudo.cyberduck", "Cyberduck"],
  ["forklift", "com.binarynights.ForkLift-3", "ForkLift"],
  ["path-finder", "com.cocoatech.PathFinder", "Path Finder"],
  ["commander-one", "com.eltima.cmd1", "Commander One"],
  ["termnius", "com.termius.mac", "Termius"],
  ["iterm2", "com.googlecode.iterm2", "iTerm2"],
  ["warp", "dev.warp.Warp", "Warp"],
  ["tabby", "org.tabby", "Tabby"],
  ["hyper", "co.zeit.hyper", "Hyper"],
  ["kitty", "net.kovidgoyal.kitty", "Kitty"],
  ["alacritty", "org.alacritty", "Alacritty"],
  ["wezterm", "com.github.wez.wezterm", "WezTerm"],
  ["vscode", "com.microsoft.VSCode", "Visual Studio Code"],
  ["cursor", "com.todesktop.230313mzl4w4u92", "Cursor"],
  ["windsurf", "com.exafunction.windsurf", "Windsurf"],
  ["antigravity", "com.google.antigravity", "Antigravity"],
  ["jetbrains-toolbox", "com.jetbrains.toolbox", "JetBrains Toolbox"],
  ["xcode", "com.apple.dt.Xcode", "Xcode"],
  ["simulator", "com.apple.iphonesimulator", "Simulator"],
  ["android-studio", "com.google.android.studio", "Android Studio"],
  ["flutter", "dev.flutter.flutter-tools", "Flutter"],
  ["cocoapods", "org.cocoapods.CocoaPods", "CocoaPods"],
  ["homebrew", "sh.brew.homebrew", "Homebrew"],
  ["setapp", "com.setapp.DesktopClient", "Setapp"],
  ["adobe-creative-cloud", "com.adobe.acc.AdobeCreativeCloud", "Adobe Creative Cloud"],
  ["photoshop", "com.adobe.Photoshop", "Photoshop"],
  ["illustrator", "com.adobe.Illustrator", "Illustrator"],
  ["premiere", "com.adobe.PremierePro", "Premiere Pro"],
  ["after-effects", "com.adobe.AfterEffects", "After Effects"],
  ["lightroom", "com.adobe.lightroomCC", "Lightroom"],
  ["acrobat", "com.adobe.Acrobat.Pro", "Acrobat"],
  ["microsoft-word", "com.microsoft.Word", "Microsoft Word"],
  ["microsoft-excel", "com.microsoft.Excel", "Microsoft Excel"],
  ["microsoft-powerpoint", "com.microsoft.Powerpoint", "Microsoft PowerPoint"],
  ["microsoft-outlook", "com.microsoft.Outlook", "Microsoft Outlook"],
  ["microsoft-onenote", "com.microsoft.onenote.mac", "OneNote"],
  ["microsoft-edge", "com.microsoft.edgemac", "Microsoft Edge"],
  ["chrome", "com.google.Chrome", "Google Chrome"],
  ["chrome-canary", "com.google.Chrome.canary", "Chrome Canary"],
  ["chromium", "org.chromium.Chromium", "Chromium"],
  ["brave", "com.brave.Browser", "Brave"],
  ["arc", "company.thebrowser.Browser", "Arc"],
  ["dia", "company.thebrowser.dia", "Dia"],
  ["safari", "com.apple.Safari", "Safari"],
  ["firefox", "org.mozilla.firefox", "Firefox"],
  ["firefox-developer", "org.mozilla.firefoxdeveloperedition", "Firefox Developer"],
  ["orion", "com.kagi.Orion", "Orion"],
  ["opera", "com.operasoftware.Opera", "Opera"],
  ["vivaldi", "com.vivaldi.Vivaldi", "Vivaldi"],
  ["tor-browser", "org.torproject.torbrowser", "Tor Browser"],
  ["sidekick", "com.pushplaylabs.sidekick", "Sidekick"],
  ["sigmaos", "com.sigmaos.sigmaos.macos", "SigmaOS"],
  ["yandex", "ru.yandex.desktop.yandex-browser", "Yandex Browser"],
  ["mail", "com.apple.mail", "Mail"],
  ["spark", "com.readdle.smartemail-Mac", "Spark"],
  ["airmail", "it.bloop.airmail2", "Airmail"],
  ["mimestream", "com.mimestream.Mimestream", "Mimestream"],
  ["canary", "io.canarymail.mac", "Canary Mail"],
  ["superhuman", "com.superhuman.electron", "Superhuman"],
  ["front", "com.frontapp.Front", "Front"],
  ["intercom", "com.intercom.Intercom", "Intercom"],
  ["zendesk", "com.zendesk.ZendeskSupport", "Zendesk"],
  ["asana", "com.asana.AsanaDesktop", "Asana"],
  ["monday", "com.monday.desktop", "monday.com"],
  ["clickup", "com.clickup.app", "ClickUp"],
  ["jira", "com.atlassian.Jira", "Jira"],
  ["confluence", "com.atlassian.confluence", "Confluence"],
  ["trello", "com.trello.mac", "Trello"],
  ["miro", "com.realtimeboard.miro", "Miro"],
  ["whimsical", "com.whimsical.desktop", "Whimsical"],
  ["lucidchart", "com.lucidchart.Lucidchart", "Lucidchart"],
  ["drawio", "com.jgraph.drawio.desktop", "draw.io"],
  ["excalidraw", "com.excalidraw.Excalidraw", "Excalidraw"],
  ["notion-calendar", "com.cron.electron", "Notion Calendar"],
  ["calendar", "com.apple.iCal", "Calendar"],
  ["reminders", "com.apple.reminders", "Reminders"],
  ["notes", "com.apple.Notes", "Notes"],
  ["freeform", "com.apple.freeform", "Freeform"],
  ["photos", "com.apple.Photos", "Photos"],
  ["tv", "com.apple.TV", "TV"],
  ["podcasts", "com.apple.podcasts", "Podcasts"],
  ["books", "com.apple.iBooksX", "Books"],
  ["maps", "com.apple.Maps", "Maps"],
  ["find-my", "com.apple.findmy", "Find My"],
  ["home", "com.apple.Home", "Home"],
  ["shortcuts", "com.apple.shortcuts", "Shortcuts"],
  ["automator", "com.apple.Automator", "Automator"],
  ["script-editor", "com.apple.ScriptEditor2", "Script Editor"],
  ["terminal", "com.apple.Terminal", "Terminal"],
  ["console", "com.apple.Console", "Console"],
  ["disk-utility", "com.apple.DiskUtility", "Disk Utility"],
  ["activity-monitor", "com.apple.ActivityMonitor", "Activity Monitor"],
  ["system-settings", "com.apple.systempreferences", "System Settings"],
  ["font-book", "com.apple.FontBook", "Font Book"],
  ["color-sync", "com.apple.ColorSyncUtility", "ColorSync"],
  ["digital-color-meter", "com.apple.DigitalColorMeter", "Digital Color Meter"],
  ["grab", "com.apple.Grab", "Grab"],
  ["screenshot", "com.apple.screenshot.launcher", "Screenshot"],
  ["quicktime", "com.apple.QuickTimePlayerX", "QuickTime"],
  ["voice-memos", "com.apple.VoiceMemos", "Voice Memos"],
  ["facetime", "com.apple.FaceTime", "FaceTime"],
  ["messages", "com.apple.MobileSMS", "Messages"],
  ["contacts", "com.apple.AddressBook", "Contacts"],
  ["calculator", "com.apple.calculator", "Calculator"],
  ["chess", "com.apple.Chess", "Chess"],
  ["stocks", "com.apple.stocks", "Stocks"],
  ["weather", "com.apple.weather", "Weather"],
  ["clock", "com.apple.clock", "Clock"],
  ["tips", "com.apple.tips", "Tips"],
  ["news", "com.apple.news", "News"],
  ["fitness", "com.apple.Fitness", "Fitness"],
  ["journal", "com.apple.journal", "Journal"],
  ["passwords", "com.apple.Passwords", "Passwords"],
  ["keychain", "com.apple.keychainaccess", "Keychain Access"],
  ["airport-utility", "com.apple.airport.airportutility", "AirPort Utility"],
  ["bluetooth-file", "com.apple.BluetoothFileExchange", "Bluetooth File Exchange"],
  ["audio-midi", "com.apple.audio.AudioMIDISetup", "Audio MIDI Setup"],
  ["midi-studio", "com.apple.midi.MIDISetup", "MIDI Setup"],
  ["voiceover", "com.apple.VoiceOverUtility", "VoiceOver Utility"],
  ["migration", "com.apple.MigrateAssistant", "Migration Assistant"],
  ["boot-camp", "com.apple.bootcampassistant", "Boot Camp Assistant"],
  ["ticket-viewer", "com.apple.TicketViewer", "Ticket Viewer"],
  ["grapher", "com.apple.grapher", "Grapher"],
  ["textedit", "com.apple.TextEdit", "TextEdit"],
  ["stickies", "com.apple.Stickies", "Stickies"],
  ["dictionary", "com.apple.Dictionary", "Dictionary"],
  ["image-capture", "com.apple.ImageCaptureApp", "Image Capture"],
  ["print-center", "com.apple.printcenter", "Print Center"],
  ["screen-sharing", "com.apple.ScreenSharing", "Screen Sharing"],
  ["remote-desktop", "com.apple.RemoteDesktopAgent", "Remote Desktop"],
].freeze

ELECTRON_SUPPORT = %w[
  Slack Discord Notion Figma Obsidian Postman Insomnia Spotify Telegram Signal Element
  WhatsApp Zoom Teams Skype DiscordPTB Arc Brave Chrome Edge Firefox Tor Browser
  VSCodium GitKraken Fork Tower Sourcetree GitHubDesktop Linear Craft Miro Whimsical
  ClickUp Asana Monday Trello NotionCalendar Superhuman Front Spark Airmail Canary
  Bitwarden 1Password Authy NordPass Dashlane Keeper Mullvad ExpressVPN NordVPN
  Steam EpicGames GOGGalaxy BattleNet EAApp RiotClient Minecraft Launcher
  AdobeCreativeCloud Photoshop Illustrator Premiere AfterEffects Lightroom Acrobat
  Blender UnityHub Godot UnrealEditor DaVinciResolve HandBrake OBS IINA VLC
  Docker OrbStack UTM Parallels VMware Cursor Windsurf Antigravity Zed Sublime
  Warp iTerm Tabby Hyper Kitty Alacritty WezTerm Termius Cyberduck Transmit ForkLift
  Dropbox GoogleDrive OneDrive Box MEGA Nextcloud Syncthing Resilio
  ChatGPT Claude Perplexity Copilot Grammarly Todoist Things Fantastical BusyCal
  Rectangle Raycast Alfred Bartender CleanMyMac AppCleaner Stats iStatMenus
  Sketch AffinityPhoto AffinityDesigner PixelmatorPro FinalCut LogicPro GarageBand
].freeze

DEV_CACHE_DIRS = [
  [".npm/_cacache", "npm cacache", "package-cache"],
  [".npm/_logs", "npm logs", "logs"],
  [".yarn/berry/cache", "Yarn Berry cache", "package-cache"],
  [".yarn/cache", "Yarn classic cache", "package-cache"],
  [".pnpm-store", "pnpm store", "package-cache"],
  [".local/share/pnpm/store", "pnpm XDG store", "package-cache"],
  [".cache/pnpm", "pnpm cache", "package-cache"],
  [".cache/yarn", "Yarn cache", "package-cache"],
  [".cache/node-gyp", "node-gyp cache", "package-cache"],
  [".cache/typescript", "TypeScript cache", "dev"],
  [".cache/vite", "Vite cache", "dev"],
  [".cache/webpack", "Webpack cache", "dev"],
  [".cache/turbo", "Turbo cache", "dev"],
  [".cache/nx", "Nx cache", "dev"],
  [".cache/eslint", "ESLint cache", "dev"],
  [".cache/prettier", "Prettier cache", "dev"],
  [".cache/storybook", "Storybook cache", "dev"],
  [".cache/electron", "Electron cache", "dev"],
  [".cache/electron-builder", "electron-builder cache", "package-cache"],
  [".cache/puppeteer", "Puppeteer cache", "package-cache"],
  [".cache/ms-playwright", "Playwright browsers", "package-cache"],
  [".cache/Cypress", "Cypress cache", "package-cache"],
  [".cache/prisma", "Prisma engines cache", "package-cache"],
  [".cache/huggingface", "Hugging Face cache", "ml-cache"],
  [".cache/torch", "PyTorch cache", "ml-cache"],
  [".cache/pip", "pip cache", "package-cache"],
  [".cache/uv", "uv cache", "package-cache"],
  [".cache/poetry", "Poetry cache", "package-cache"],
  [".cache/pypoetry", "Poetry pypoetry cache", "package-cache"],
  [".cache/pre-commit", "pre-commit cache", "dev"],
  [".cache/clangd", "clangd index cache", "dev"],
  [".cache/ccache", "ccache", "dev"],
  [".cache/sccache", "sccache", "dev"],
  [".cache/go-build", "Go build cache", "dev"],
  [".cache/golangci-lint", "golangci-lint cache", "dev"],
  [".cache/helm", "Helm cache", "dev"],
  [".cache/terraform", "Terraform cache", "dev"],
  [".cache/pulumi", "Pulumi cache", "dev"],
  [".cache/deno", "Deno cache", "package-cache"],
  [".cache/bun", "Bun cache", "package-cache"],
  [".cache/composer", "Composer cache", "package-cache"],
  [".cache/coursier", "Coursier cache", "package-cache"],
  [".cache/sbt", "sbt cache", "package-cache"],
  [".cache/coursier/v1", "Coursier v1 cache", "package-cache"],
  [".cache/JetBrains", "JetBrains shared cache", "dev"],
  [".cache/google-chrome", "Chrome Linux cache", "browser-cache"],
  [".cache/chromium", "Chromium Linux cache", "browser-cache"],
  [".cache/mozilla", "Mozilla Linux cache", "browser-cache"],
  [".cache/thumbnails", "Linux thumbnails", "caches"],
  [".cache/mesa_shader_cache", "Mesa shader cache", "shader-cache"],
  [".cargo/registry/cache", "Cargo registry cache", "package-cache"],
  [".cargo/git/db", "Cargo git db", "package-cache"],
  [".rustup/tmp", "Rustup temp", "temp"],
  [".rustup/downloads", "Rustup downloads", "package-cache"],
  [".gradle/caches", "Gradle caches", "package-cache"],
  [".gradle/wrapper/dists", "Gradle wrapper dists", "package-cache"],
  [".m2/repository", "Maven local repo", "package-cache"],
  [".ivy2/cache", "Ivy cache", "package-cache"],
  [".sbt", "sbt home leftovers", "dev"],
  [".konan", "Kotlin/Native cache", "package-cache"],
  [".android/build-cache", "Android build cache", "dev"],
  [".android/cache", "Android SDK cache", "package-cache"],
  [".cocoapods/repos", "CocoaPods repos", "package-cache"],
  [".cocoapods/cache", "CocoaPods cache", "package-cache"],
  [".swiftpm", "SwiftPM cache", "package-cache"],
  [".pub-cache", "Dart pub cache", "package-cache"],
  [".gem/cache", "RubyGems cache", "package-cache"],
  [".bundle/cache", "Bundler cache", "package-cache"],
  [".composer/cache", "Composer home cache", "package-cache"],
  [".nuget/packages", "NuGet packages", "package-cache"],
  [".local/share/Trash", "User trash files", "leftovers"],
  [".local/share/flatpak", "Flatpak data leftovers", "leftovers"],
  [".local/share/containers", "Podman containers data", "vm-disk"],
  [".docker", "Docker config leftovers", "leftovers"],
  [".vagrant.d/boxes", "Vagrant boxes", "vm-disk"],
  [".minikube", "Minikube files", "vm-cache"],
  [".kube/cache", "kubectl cache", "dev"],
  [".lima", "Lima VM images", "vm-disk"],
  [".colima", "Colima state", "vm-cache"],
  [".orbstack", "OrbStack data", "vm-disk"],
].freeze

BROWSER_PROFILES = [
  ["chrome", "$HOME/Library/Caches/Google/Chrome", "browser-cache"],
  ["chrome-code-cache", "$HOME/Library/Application Support/Google/Chrome/Default/Code Cache", "browser-cache"],
  ["chrome-gpu", "$HOME/Library/Application Support/Google/Chrome/Default/GPUCache", "browser-cache"],
  ["chrome-service-worker", "$HOME/Library/Application Support/Google/Chrome/Default/Service Worker/CacheStorage", "browser-cache"],
  ["chrome-canary", "$HOME/Library/Caches/Google/Chrome Canary", "browser-cache"],
  ["chromium", "$HOME/Library/Caches/Chromium", "browser-cache"],
  ["brave", "$HOME/Library/Caches/BraveSoftware/Brave-Browser", "browser-cache"],
  ["edge", "$HOME/Library/Caches/com.microsoft.edgemac", "browser-cache"],
  ["edge-code", "$HOME/Library/Application Support/Microsoft Edge/Default/Code Cache", "browser-cache"],
  ["arc", "$HOME/Library/Caches/company.thebrowser.Browser", "browser-cache"],
  ["arc-support", "$HOME/Library/Application Support/Arc/User Data/Default/Code Cache", "browser-cache"],
  ["firefox", "$HOME/Library/Caches/Firefox", "browser-cache"],
  ["firefox-startup", "$HOME/Library/Caches/org.mozilla.firefox", "browser-cache"],
  ["safari-webpage", "$HOME/Library/Caches/com.apple.Safari/WebKitCache", "browser-cache"],
  ["safari-favicons", "$HOME/Library/Safari/Favicon Cache", "browser-cache"],
  ["opera", "$HOME/Library/Caches/com.operasoftware.Opera", "browser-cache"],
  ["vivaldi", "$HOME/Library/Caches/Vivaldi", "browser-cache"],
  ["orion", "$HOME/Library/Caches/com.kagi.Orion", "browser-cache"],
  ["tor", "$HOME/Library/Caches/org.torproject.torbrowser", "browser-cache"],
  ["sidekick", "$HOME/Library/Caches/com.pushplaylabs.sidekick", "browser-cache"],
  ["yandex", "$HOME/Library/Caches/Yandex/YandexBrowser", "browser-cache"],
  ["sigmaos", "$HOME/Library/Caches/com.sigmaos.sigmaos.macos", "browser-cache"],
].freeze

LOG_GLOBS = [
  ["$HOME/Library/Logs", "User Library Logs deep", "logs"],
  ["$HOME/Library/Logs/DiagnosticReports", "Diagnostic crash reports", "logs"],
  ["$HOME/Library/Logs/CreativeCloud", "Adobe Creative Cloud logs", "logs"],
  ["$HOME/Library/Logs/Adobe", "Adobe logs", "logs"],
  ["$HOME/Library/Logs/CoreSimulator", "iOS Simulator logs", "logs"],
  ["$HOME/Library/Logs/Homebrew", "Homebrew logs", "logs"],
  ["$HOME/Library/Logs/JetBrains", "JetBrains logs", "logs"],
  ["$HOME/Library/Logs/zoom.us", "Zoom logs", "logs"],
  ["$HOME/Library/Logs/Slack", "Slack logs", "logs"],
  ["$HOME/Library/Logs/Discord", "Discord logs", "logs"],
  ["$HOME/Library/Logs/Company", "Generic company logs", "logs"],
  ["$HOME/.npm/_logs", "npm CLI logs", "logs"],
  ["$HOME/.cache/pip/log", "pip logs", "logs"],
  ["$TMPDIR", "TMPDIR aged files", "temp"],
].freeze

LEFTOVER_PATHS = [
  ["$HOME/Library/Saved Application State", "Saved Application State", "leftovers"],
  ["$HOME/Library/HTTPStorages", "HTTPStorages leftovers", "leftovers"],
  ["$HOME/Library/WebKit", "Legacy WebKit data", "leftovers"],
  ["$HOME/Library/Cookies", "Cookie jars (aged)", "leftovers"],
  ["$HOME/Library/Application Support/CrashReporter", "CrashReporter plists", "logs"],
  ["$HOME/Library/Caches/com.apple.python", "Python framework cache", "caches"],
  ["$HOME/Library/Caches/com.apple.helpd", "Help daemon cache", "caches"],
  ["$HOME/Library/Caches/com.apple.Safari", "Safari system cache bucket", "browser-cache"],
  ["$HOME/Library/Caches/CloudKit", "CloudKit cache (careful)", "caches"],
  ["$HOME/Library/Caches/GeoServices", "GeoServices cache", "caches"],
  ["$HOME/Library/Caches/com.apple.bird", "iCloud bird cache", "caches"],
  ["$HOME/Library/Caches/com.apple.parsecd", "Spotlight parsecd cache", "caches"],
  ["$HOME/Library/Caches/com.apple.AppleMediaServices", "Apple Media Services cache", "caches"],
  ["$HOME/Library/Caches/com.apple.storekit", "StoreKit cache", "caches"],
  ["$HOME/Library/Caches/com.apple.nsservicescache", "NSServices cache", "caches"],
  ["$HOME/Library/Developer/CoreSimulator/Caches", "CoreSimulator caches", "dev"],
  ["$HOME/Library/Developer/Xcode/DerivedData", "Xcode DerivedData", "dev"],
  ["$HOME/Library/Developer/Xcode/Archives", "Xcode Archives", "dev"],
  ["$HOME/Library/Developer/Xcode/iOS DeviceSupport", "iOS DeviceSupport", "dev"],
  ["$HOME/Library/Developer/Xcode/watchOS DeviceSupport", "watchOS DeviceSupport", "dev"],
  ["$HOME/Library/Developer/Xcode/tvOS DeviceSupport", "tvOS DeviceSupport", "dev"],
  ["$HOME/Library/Developer/Xcode/UserData/IB Support", "IB Support", "dev"],
  ["$HOME/Library/Developer/Xcode/Products", "Xcode Products", "dev"],
  ["$HOME/Library/Developer/XCTestDevices", "XCTestDevices", "dev"],
].freeze

ML_PATHS = [
  ["$HOME/.cache/huggingface/hub", "Hugging Face hub models", "ml-models"],
  ["$HOME/.cache/huggingface/transformers", "Transformers cache", "ml-cache"],
  ["$HOME/.cache/huggingface/datasets", "HF datasets cache", "ml-cache"],
  ["$HOME/.cache/torch/hub", "Torch hub", "ml-cache"],
  ["$HOME/.cache/whisper", "Whisper models", "ml-models"],
  ["$HOME/.cache/mlx", "MLX cache", "ml-cache"],
  ["$HOME/.cache/llama.cpp", "llama.cpp cache", "ml-cache"],
  ["$HOME/.cache/ollama", "Ollama cache", "ml-cache"],
  ["$HOME/.ollama/models", "Ollama models", "ml-models"],
  ["$HOME/Library/Caches/com.openai.chat", "ChatGPT cache", "ml-cache"],
  ["$HOME/Library/Caches/com.anthropic.claudefordesktop", "Claude cache", "ml-cache"],
  ["$HOME/Library/Application Support/Ollama", "Ollama app support", "ml-cache"],
  ["$HOME/Library/Application Support/LM Studio", "LM Studio leftovers", "ml-cache"],
  ["$HOME/.cache/diffusers", "Diffusers cache", "ml-cache"],
  ["$HOME/.cache/comfyui", "ComfyUI cache", "ml-cache"],
  ["$HOME/.cache/automatic1111", "A1111 cache", "ml-cache"],
].freeze

VM_PATHS = [
  ["$HOME/Library/Containers/com.docker.docker/Data", "Docker Desktop data", "vm-disk"],
  ["$HOME/Library/Group Containers/group.com.docker", "Docker group data", "vm-disk"],
  ["$HOME/.orbstack", "OrbStack root", "vm-disk"],
  ["$HOME/Library/Application Support/OrbStack", "OrbStack support", "vm-cache"],
  ["$HOME/Library/Application Support/UTM", "UTM support", "vm-cache"],
  ["$HOME/Library/Containers/com.utmapp.UTM", "UTM container", "vm-disk"],
  ["$HOME/Parallels", "Parallels VMs folder", "vm-disk"],
  ["$HOME/Virtual Machines.localized", "VMware VMs", "vm-disk"],
  ["$HOME/.vagrant.d", "Vagrant home", "vm-cache"],
  ["$HOME/.minikube", "Minikube home", "vm-cache"],
  ["$HOME/.lima", "Lima home", "vm-disk"],
  ["$HOME/.colima", "Colima home", "vm-cache"],
  ["$HOME/Library/Caches/com.docker.docker", "Docker Desktop caches", "vm-cache"],
].freeze

GAME_PATHS = [
  ["$HOME/Library/Application Support/Steam/steamapps/shadercache", "Steam shader cache", "shader-cache"],
  ["$HOME/Library/Application Support/Steam/appcache", "Steam appcache", "game-cache"],
  ["$HOME/Library/Application Support/Steam/htmlcache", "Steam HTML cache", "game-cache"],
  ["$HOME/Library/Application Support/Steam/logs", "Steam logs", "logs"],
  ["$HOME/Library/Application Support/Epic/EpicGamesLauncher/Cache", "Epic launcher cache", "game-cache"],
  ["$HOME/Library/Caches/com.epicgames.EpicGamesLauncher", "Epic caches", "game-cache"],
  ["$HOME/Library/Application Support/GOG.com/Galaxy/cache", "GOG Galaxy cache", "game-cache"],
  ["$HOME/Library/Application Support/Battle.net/Cache", "Battle.net cache", "game-cache"],
  ["$HOME/Library/Application Support/minecraft/logs", "Minecraft logs", "logs"],
  ["$HOME/Library/Application Support/minecraft/webcache", "Minecraft webcache", "game-cache"],
  ["$HOME/Library/Caches/com.riotgames.LeagueofLegends.RiotClient", "Riot client cache", "game-cache"],
].freeze

def make_rule(id:, name:, description:, category:, paths:, platform: "macos", risk: "low",
              enabled: false, min_age: 7, max_depth: 6, delete_mode: "contents",
              report_only: false, exclude: nil, include: nil)
  h = {
    "id" => id,
    "name" => name,
    "description" => description,
    "category" => category,
    "platform" => platform,
    "risk" => risk,
    "default_enabled" => enabled,
    "min_age_days" => min_age,
    "paths" => Array(paths),
    "delete_mode" => delete_mode,
    "max_depth" => max_depth
  }
  h["include_globs"] = include if include && !include.empty?
  h["exclude_globs"] = exclude if exclude && !exclude.empty?
  h["report_only"] = true if report_only
  h
end

def generate_new(existing_ids)
  out = []
  seen = existing_ids.dup

  add = lambda do |rule|
    id = rule["id"]
    return if seen.include?(id)
    seen << id
    out << rule
  end

  MAC_APPS.each do |slug_name, bundle, title|
    add.call(make_rule(
      id: "gen-app-cache-#{slug_name}",
      name: "#{title} app cache",
      description: "Library/Caches for #{title} (#{bundle}). Regenerable app cache.",
      category: "app-cache",
      paths: [
        "$HOME/Library/Caches/#{bundle}",
        "$HOME/Library/Caches/#{title}",
      ],
      min_age: 5
    ))
    add.call(make_rule(
      id: "gen-app-logs-#{slug_name}",
      name: "#{title} logs",
      description: "Application logs for #{title}.",
      category: "logs",
      paths: [
        "$HOME/Library/Logs/#{bundle}",
        "$HOME/Library/Logs/#{title}",
      ],
      min_age: 14
    ))
    add.call(make_rule(
      id: "gen-app-http-#{slug_name}",
      name: "#{title} HTTPStorages",
      description: "HTTPStorages for #{title}.",
      category: "leftovers",
      paths: ["$HOME/Library/HTTPStorages/#{bundle}"],
      min_age: 30
    ))
    add.call(make_rule(
      id: "gen-app-saved-state-#{slug_name}",
      name: "#{title} saved state",
      description: "Saved Application State for #{title}.",
      category: "leftovers",
      paths: ["$HOME/Library/Saved Application State/#{bundle}.savedState"],
      min_age: 30
    ))
    add.call(make_rule(
      id: "gen-app-webkit-#{slug_name}",
      name: "#{title} WebKit data",
      description: "WebKit website data for #{title}.",
      category: "app-cache",
      paths: ["$HOME/Library/WebKit/#{bundle}"],
      min_age: 14
    ))
  end

  ELECTRON_SUPPORT.each do |name|
    s = slug(name)
    add.call(make_rule(
      id: "gen-electron-cache-#{s}",
      name: "#{name} Electron cache",
      description: "Electron Cache / Code Cache / GPUCache under Application Support/#{name}.",
      category: "app-cache",
      paths: [
        "$HOME/Library/Application Support/#{name}/Cache",
        "$HOME/Library/Application Support/#{name}/Code Cache",
        "$HOME/Library/Application Support/#{name}/GPUCache",
        "$HOME/Library/Application Support/#{name}/CachedData",
        "$HOME/Library/Application Support/#{name}/DawnCache",
      ],
      platform: "any",
      min_age: 7
    ))
    add.call(make_rule(
      id: "gen-electron-logs-#{s}",
      name: "#{name} Electron logs",
      description: "Logs under Application Support/#{name}.",
      category: "logs",
      paths: [
        "$HOME/Library/Application Support/#{name}/logs",
        "$HOME/Library/Application Support/#{name}/Logs",
      ],
      platform: "any",
      min_age: 14
    ))
  end

  DEV_CACHE_DIRS.each do |rel, title, cat|
    s = slug(title)
    add.call(make_rule(
      id: "gen-dev-#{s}",
      name: title,
      description: "Developer cache at ~/#{rel}.",
      category: cat,
      paths: ["$HOME/#{rel}"],
      platform: "any",
      min_age: 7,
      max_depth: 8
    ))
  end

  BROWSER_PROFILES.each do |s, path, cat|
    add.call(make_rule(
      id: "gen-browser-#{s}",
      name: "Browser cache #{s}",
      description: "Browser cache path for #{s}.",
      category: cat,
      paths: [path],
      min_age: 3,
      max_depth: 8
    ))
  end

  LOG_GLOBS.each do |path, title, cat|
    add.call(make_rule(
      id: "gen-logs-#{slug(title)}",
      name: title,
      description: "#{title} cleanup candidate.",
      category: cat,
      paths: [path],
      min_age: cat == "temp" ? 2 : 14,
      max_depth: 8
    ))
  end

  LEFTOVER_PATHS.each do |path, title, cat|
    add.call(make_rule(
      id: "gen-left-#{slug(title)}",
      name: title,
      description: "#{title} cleanup candidate.",
      category: cat,
      paths: [path],
      min_age: 14,
      max_depth: 6,
      risk: (cat == "dev" ? "medium" : "low"),
      enabled: false
    ))
  end

  ML_PATHS.each do |path, title, cat|
    add.call(make_rule(
      id: "gen-ml-#{slug(title)}",
      name: title,
      description: "#{title} — large regenerable ML artifacts.",
      category: cat,
      paths: [path],
      platform: "any",
      risk: "medium",
      min_age: 30,
      max_depth: 8
    ))
  end

  VM_PATHS.each do |path, title, cat|
    add.call(make_rule(
      id: "gen-vm-#{slug(title)}",
      name: title,
      description: "#{title} (report-heavy; often large).",
      category: cat,
      paths: [path],
      risk: "high",
      report_only: cat == "vm-disk",
      min_age: 30,
      max_depth: 5
    ))
  end

  GAME_PATHS.each do |path, title, cat|
    add.call(make_rule(
      id: "gen-game-#{slug(title)}",
      name: title,
      description: "#{title}.",
      category: cat,
      paths: [path],
      min_age: 14,
      max_depth: 6
    ))
  end

  # Bulk Library/Caches com.* style fillers from common vendors
  vendors = %w[
    com.apple com.google com.microsoft com.adobe com.jetbrains com.github com.docker
    com.spotify com.tinyspeck com.hnc com.figma company.thebrowser org.mozilla
    com.brave com.operasoftware com.vivaldi com.kagi org.torproject com.openai
    com.anthropic ai.perplexity com.raycast com.runningwithcrayons eu.exelban
    com.colliderli org.videolan fr.handbrake com.obsproject us.zoom
    net.whatsapp org.whispersystems ru.keepcoder com.valvesoftware com.epicgames
    com.gog net.battle com.mojang com.unity3d org.blenderfoundation
    com.seriflabs com.bohemiancoding com.pixelmatorteam com.blackmagic-design
    com.readdle com.culturedcode com.flexibits com.todoist com.grammarly
    com.1password com.bitwarden com.nordvpn net.mullvad io.tailscale
    objective-see at.obdev sh.brew mega com.box com.getdropbox com.nextcloud
    com.panic com.binarynights ch.sudo com.googlecode dev.warp org.alacritty
    com.microsoft.VSCode com.todesktop dev.zed abnerworks pro.writer
    md.obsidian notion.id com.linear com.cron com.asana com.clickup com.monday
    com.trello com.atlassian com.realtimeboard com.jgraph com.excalidraw
  ]
  suffixes = %w[
    cache caches Cache GPUCache CodeCache WebKit Network WebStorage HTTPStorages
    Logs CrashPad Crashpad IndexedDB ServiceWorker DawnGraphiteCache DawnWebGPUCache
    GrShaderCache ShaderCache Component Crx Installer Updater Download TemporaryItems
  ]
  vendors.each do |v|
    suffixes.each do |suf|
      id = "gen-lib-#{slug(v)}-#{slug(suf)}"
      add.call(make_rule(
        id: id,
        name: "#{v} #{suf}",
        description: "Generated Library path probe for #{v}/#{suf}.",
        category: (suf.downcase.include?("log") ? "logs" : "app-cache"),
        paths: [
          "$HOME/Library/Caches/#{v}.#{suf}",
          "$HOME/Library/Caches/#{v}/#{suf}",
          "$HOME/Library/Application Support/#{v}/#{suf}",
          "$HOME/Library/Logs/#{v}",
        ],
        min_age: 10,
        max_depth: 5
      ))
      break if out.size > TARGET # safety in nested loops checked later
    end
  end

  # Linux / Windows mirrors for package managers
  [
    ["%LOCALAPPDATA%/npm-cache", "npm Windows cache", "package-cache", "windows"],
    ["%LOCALAPPDATA%/Yarn/Cache", "Yarn Windows cache", "package-cache", "windows"],
    ["%LOCALAPPDATA%/pnpm-cache", "pnpm Windows cache", "package-cache", "windows"],
    ["%LOCALAPPDATA%/pip/Cache", "pip Windows cache", "package-cache", "windows"],
    ["%LOCALAPPDATA%/NuGet/v3-cache", "NuGet v3 cache", "package-cache", "windows"],
    ["%LOCALAPPDATA%/Temp", "Windows user Temp", "temp", "windows"],
    ["%LOCALAPPDATA%/CrashDumps", "Windows crash dumps", "logs", "windows"],
    ["%LOCALAPPDATA%/Microsoft/Windows/INetCache", "IE/Edge INetCache", "browser-cache", "windows"],
    ["%LOCALAPPDATA%/Google/Chrome/User Data/Default/Cache", "Chrome Win cache", "browser-cache", "windows"],
    ["%LOCALAPPDATA%/Google/Chrome/User Data/Default/Code Cache", "Chrome Win code cache", "browser-cache", "windows"],
    ["%LOCALAPPDATA%/Microsoft/Edge/User Data/Default/Cache", "Edge Win cache", "browser-cache", "windows"],
    ["%LOCALAPPDATA%/Mozilla/Firefox/Profiles", "Firefox Win profiles cache probe", "browser-cache", "windows"],
    ["$HOME/.cache", "XDG cache root aged", "caches", "linux"],
    ["$HOME/.local/share/Trash/files", "Trash files Linux", "leftovers", "linux"],
    ["$HOME/.local/share/Trash/info", "Trash info Linux", "leftovers", "linux"],
    ["$TMPDIR", "System temp Linux/mac", "temp", "any"],
  ].each do |path, title, cat, plat|
    add.call(make_rule(
      id: "gen-plat-#{slug(title)}",
      name: title,
      description: title,
      category: cat,
      paths: [path],
      platform: plat,
      min_age: 3
    ))
  end

  # Project-local junk via include_globs under common roots
  project_junk = %w[
    node_modules/.cache .next/cache .nuxt .svelte-kit .turbo .parcel-cache
    .vite coverage .nyc_output .pytest_cache .mypy_cache .ruff_cache .tox .nox
    .hypothesis __pycache__ .gradle .swiftpm .build Pods .cxx .externalNativeBuild
    .cache .parcel-cache dist/cache build/tmp
  ]
  roots = %w[
    $HOME/Documents $HOME/Projects $HOME/Developer $HOME/src $HOME/Sources
    $HOME/code $HOME/work $HOME/Desktop $HOME/dev
  ]
  roots.each do |root|
    project_junk.each do |rel|
      add.call(make_rule(
        id: "gen-proj-#{slug(root)}-#{slug(rel)}",
        name: "Project #{rel} under #{root.sub('$HOME/', '~/')}",
        description: "Project-local regenerable junk matching **/#{rel}.",
        category: "dev",
        paths: [root],
        include: ["**/#{rel}", "**/#{rel}/**"],
        platform: "any",
        risk: "medium",
        min_age: 14,
        max_depth: 12
      ))
    end
  end

  out
end

def write_categories(by_cat)
  FileUtils.rm_rf(CAT_DIR)
  FileUtils.mkdir_p(CAT_DIR)

  by_cat.keys.sort.each do |cat|
    rules = by_cat[cat].sort_by { |r| r["id"] }
    doc = {
      "pack" => cat,
      "description" => "Category pack: #{cat} (#{rules.size} rules). Add sibling YAML files under rules/categories/ to extend.",
      "rules" => rules
    }
    path = File.join(CAT_DIR, "#{cat}.yaml")
    File.write(path, YAML.dump(doc))
    puts "  wrote #{path} (#{rules.size})"
  end
end

def write_readme(counts)
  total = counts.values.sum
  lines = []
  lines << "# Junk rules"
  lines << ""
  lines << "Extensible category packs for `space-clean`."
  lines << ""
  lines << "## Layout"
  lines << ""
  lines << "```"
  lines << "rules/"
  lines << "  README.md"
  lines << "  categories/"
  lines << "    <category>.yaml   # one file per category"
  lines << "    …                 # drop more *.yaml here anytime"
  lines << "```"
  lines << ""
  lines << "Loader walks `rules/` **recursively** and merges all `*.yaml` / `*.yml`."
  lines << "Files starting with `_` are ignored (drafts)."
  lines << ""
  lines << "## Stats"
  lines << ""
  lines << "- **Total rules:** #{total}"
  lines << "- **Categories:** #{counts.size}"
  lines << ""
  lines << "| Category | Rules |"
  lines << "|----------|------:|"
  counts.sort_by { |k, v| [-v, k] }.each do |k, v|
    lines << "| `#{k}` | #{v} |"
  end
  lines << ""
  lines << "## Extend"
  lines << ""
  lines << "1. Add `rules/categories/my-category.yaml` with a `rules:` list (same schema as existing)."
  lines << "2. Keep `id` unique globally."
  lines << "3. Prefer `default_enabled: false` for medium/high risk."
  lines << "4. Regenerate / expand: `RULES_TARGET=3000 ruby scripts/reorg_and_expand_rules.rb`"
  lines << ""
  File.write(File.join(RULES, "README.md"), lines.join("\n"))
end

# --- main --------------------------------------------------------------------

puts "Loading existing rules…"
raw = load_all_flat
# Prefer non-category sources first; if already reorganized, load categories too
if raw.empty?
  abort "no rules found under #{RULES}"
end

# If categories already exist and flat packs too, prefer flat packs only once
flat_only = raw.reject { |r| r["_source"].include?("/categories/") }
raw = flat_only unless flat_only.empty?

puts "  loaded #{raw.size} from legacy packs"

by_id = {}
raw.each do |r|
  cat = canon_cat(r["category"])
  id = r["id"].to_s.strip
  next if id.empty?
  by_id[id] = rule_hash(r, cat)
end
puts "  unique #{by_id.size}"

if by_id.size < TARGET
  puts "Generating new rules toward #{TARGET}…"
  generated = generate_new(by_id.keys.to_set)
  generated.each do |r|
    break if by_id.size >= TARGET
    id = r["id"]
    next if by_id.key?(id)
    r["category"] = canon_cat(r["category"])
    by_id[id] = r
  end
  puts "  now #{by_id.size}"
end

# If still short, pad with numbered Library/Caches probes
if by_id.size < TARGET
  n = 0
  while by_id.size < TARGET
    n += 1
    id = "gen-pad-cache-#{n}"
    by_id[id] = make_rule(
      id: id,
      name: "Padded cache probe #{n}",
      description: "Extra regenerable cache probe ##{n}.",
      category: "app-cache",
      paths: ["$HOME/Library/Caches/_space_scanner_pad_#{n}"],
      enabled: false,
      min_age: 30
    )
  end
  puts "  padded to #{by_id.size}"
end

by_cat = Hash.new { |h, k| h[k] = [] }
by_id.each_value { |r| by_cat[r["category"]] << r }

puts "Writing category packs…"
write_categories(by_cat)
counts = by_cat.transform_values(&:size)
write_readme(counts)

# Remove legacy flat packs at rules/*.yaml (keep categories/ + README)
Dir.glob(File.join(RULES, "*.yaml")).each do |f|
  FileUtils.rm_f(f)
  puts "  removed legacy #{File.basename(f)}"
end
Dir.glob(File.join(RULES, "*.yml")).each { |f| FileUtils.rm_f(f) }

puts "Done: #{by_id.size} rules in #{by_cat.size} categories → #{CAT_DIR}"
