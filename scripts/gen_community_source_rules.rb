#!/usr/bin/env ruby
# frozen_string_literal: true

# Curated junk paths from public cleaners (BleachBit, mac-cleanup-sh,
# Winapp2.ini, Arch Wiki, Gradle/SO, Homebrew docs). Skips IDs already in packs.

require "yaml"
require "set"

ROOT = File.expand_path("..", __dir__)
RULES = File.join(ROOT, "rules")
OUT = File.join(RULES, "categories", "community-sources.yaml")

def load_ids
  ids = Set.new
  Dir.glob(File.join(RULES, "**", "*.{yaml,yml}")).each do |f|
    next if File.basename(f).start_with?("_")
    next if File.expand_path(f) == File.expand_path(OUT)
    data = YAML.load_file(f) || {}
    (data["rules"] || []).each { |r| ids << r["id"].to_s }
  end
  ids
end

def r(id:, name:, description:, category:, platform:, paths:,
      risk: "low", enabled: false, min_age: 7, max_depth: 6,
      delete_mode: "contents", report_only: false, source:)
  h = {
    "id" => id,
    "name" => name,
    "description" => "#{description} Source: #{source}.",
    "category" => category,
    "platform" => platform,
    "risk" => risk,
    "default_enabled" => enabled,
    "min_age_days" => min_age,
    "paths" => Array(paths).uniq,
    "delete_mode" => delete_mode,
    "max_depth" => max_depth
  }
  h["report_only"] = true if report_only
  h
end

MAC = "macos"
LIN = "linux"
WIN = "windows"

# Sources:
# BB = BleachBit cleaners XML (github.com/bleachbit/bleachbit)
# MC = mac-cleanup-sh (github.com/mac-cleanup/mac-cleanup-sh)
# W2 = Winapp2.ini community (github.com/MoscaDotTo/Winapp2)
# AW = Arch Wiki System maintenance / pacman cache
# SO = Stack Overflow Gradle cache locations
# HB = Homebrew cache docs

rules = []

# --- mac-cleanup-sh ---
rules << r(id: "src-mac-steam-transient", name: "Steam caches logs temp downloads",
  description: "Steam appcache, depot cache, shader cache, logs, temp and download folders. Not installed games in steamapps/common.",
  category: "game-cache", platform: MAC,
  paths: [
    "$HOME/Library/Application Support/Steam/appcache",
    "$HOME/Library/Application Support/Steam/depotcache",
    "$HOME/Library/Application Support/Steam/logs",
    "$HOME/Library/Application Support/Steam/steamapps/shadercache",
    "$HOME/Library/Application Support/Steam/steamapps/temp",
    "$HOME/Library/Application Support/Steam/steamapps/download"
  ], source: "mac-cleanup-sh Steam section")

rules << r(id: "src-mac-minecraft-logs-webcache", name: "Minecraft logs crash reports webcache",
  description: "Minecraft launcher logs, crash-reports and webcache. Not worlds or screenshots.",
  category: "logs", platform: MAC,
  paths: [
    "$HOME/Library/Application Support/minecraft/logs",
    "$HOME/Library/Application Support/minecraft/crash-reports",
    "$HOME/Library/Application Support/minecraft/webcache",
    "$HOME/Library/Application Support/minecraft/webcache2",
    "$HOME/Library/Application Support/minecraft/.mixin.out"
  ], source: "mac-cleanup-sh Minecraft section")

rules << r(id: "src-mac-lunarclient", name: "Lunar Client caches and logs",
  description: "Lunar Client game/launcher caches and logs. Not accounts.",
  category: "game-cache", platform: MAC,
  paths: [
    "$HOME/.lunarclient/game-cache",
    "$HOME/.lunarclient/launcher-cache",
    "$HOME/.lunarclient/logs"
  ], source: "mac-cleanup-sh Lunar Client")

rules << r(id: "src-mac-teams-classic-electron", name: "Classic Microsoft Teams Electron caches",
  description: "IndexedDB/Cache/Code Cache/GPU/blob/tmp for classic Teams. Not chat history databases outside Cache.",
  category: "app-cache", platform: MAC, risk: "medium",
  paths: [
    "$HOME/Library/Application Support/Microsoft/Teams/IndexedDB",
    "$HOME/Library/Application Support/Microsoft/Teams/Cache",
    "$HOME/Library/Application Support/Microsoft/Teams/Application Cache",
    "$HOME/Library/Application Support/Microsoft/Teams/Code Cache",
    "$HOME/Library/Application Support/Microsoft/Teams/blob_storage",
    "$HOME/Library/Application Support/Microsoft/Teams/gpucache",
    "$HOME/Library/Application Support/Microsoft/Teams/tmp",
    "$HOME/Library/Application Support/Microsoft/Teams/watchdog"
  ], source: "mac-cleanup-sh Microsoft Teams")

rules << r(id: "src-mac-drivefs-content-cache", name: "Google Drive for desktop content cache",
  description: "DriveFS content_cache is regenerable streamed file cache, not My Drive originals.",
  category: "media-cache", platform: MAC, risk: "medium",
  paths: ["$HOME/Library/Application Support/Google/DriveFS"],
  max_depth: 5, source: "mac-cleanup-sh Google Drive File Stream")

rules << r(id: "src-mac-dropbox-cache", name: "Dropbox .dropbox.cache",
  description: "Dropbox client transfer cache. Not synced files.",
  category: "caches", platform: MAC,
  paths: ["$HOME/Dropbox/.dropbox.cache"], source: "mac-cleanup-sh Dropbox")

rules << r(id: "src-mac-adobe-media-cache", name: "Adobe Common Media Cache Files",
  description: "Premiere/AE/Media Encoder peak/conformed media cache. Rebuilds on next open.",
  category: "media-cache", platform: MAC, risk: "medium",
  paths: ["$HOME/Library/Application Support/Adobe/Common/Media Cache Files"],
  source: "mac-cleanup-sh Adobe Cache")

rules << r(id: "src-mac-chrome-application-cache", name: "Chrome Application Cache",
  description: "Legacy Chrome Application Cache under Default profile. Not bookmarks or passwords.",
  category: "browser-cache", platform: MAC,
  paths: ["$HOME/Library/Application Support/Google/Chrome/Default/Application Cache"],
  source: "mac-cleanup-sh Google Chrome")

rules << r(id: "src-mac-coresimulator-logs", name: "CoreSimulator logs",
  description: "iOS Simulator diagnostic logs. Not device data.",
  category: "logs", platform: MAC,
  paths: ["$HOME/Library/Logs/CoreSimulator"], source: "mac-cleanup-sh system logs")

rules << r(id: "src-mac-xcode-ios-device-logs", name: "Xcode iOS Device Logs",
  description: "Copied device console logs in Xcode. Regenerable.",
  category: "logs", platform: MAC,
  paths: ["$HOME/Library/Developer/Xcode/iOS Device Logs"],
  source: "mac-cleanup-sh Xcode")

rules << r(id: "src-mac-poetry-cache", name: "Poetry cache",
  description: "pypoetry artifact cache. Virtualenvs live elsewhere.",
  category: "package-cache", platform: MAC,
  paths: ["$HOME/Library/Caches/pypoetry", "$HOME/.cache/pypoetry"],
  source: "mac-cleanup-sh Poetry")

rules << r(id: "src-mac-android-sdk-cache", name: "Android SDK user cache",
  description: "~/.android/cache only, not AVDs or SDK platforms.",
  category: "dev", platform: MAC,
  paths: ["$HOME/.android/cache"], source: "mac-cleanup-sh Android cache")

rules << r(id: "src-mac-gradle-caches", name: "Gradle dependency caches",
  description: "~/.gradle/caches downloaded JARs and transforms. Not gradle.properties.",
  category: "package-cache", platform: MAC, min_age: 14,
  paths: ["$HOME/.gradle/caches", "$HOME/.gradle/daemon", "$HOME/.gradle/native", "$HOME/.gradle/wrapper/dists"],
  source: "mac-cleanup-sh + StackOverflow Gradle cache")

rules << r(id: "src-mac-composer-cache", name: "Composer PHP cache",
  description: "Composer package cache (composer clearcache).",
  category: "package-cache", platform: MAC,
  paths: ["$HOME/Library/Caches/composer", "$HOME/.composer/cache"],
  source: "mac-cleanup-sh composer")

rules << r(id: "src-mac-cocoapods-cache", name: "CocoaPods cache",
  description: "pod cache clean --all target. Not Podfile.lock.",
  category: "package-cache", platform: MAC,
  paths: ["$HOME/Library/Caches/CocoaPods"], source: "mac-cleanup-sh CocoaPods")

rules << r(id: "src-mac-yarn-cache", name: "Yarn cache",
  description: "yarn cache clean target on macOS.",
  category: "package-cache", platform: MAC,
  paths: ["$HOME/Library/Caches/Yarn", "$HOME/Library/Caches/yarn"],
  source: "mac-cleanup-sh Yarn")

rules << r(id: "src-mac-pnpm-store", name: "pnpm content-addressable store",
  description: "pnpm store prune target. Packages re-fetch on next install.",
  category: "package-cache", platform: MAC, risk: "medium", min_age: 14,
  paths: ["$HOME/.pnpm-store", "$HOME/Library/pnpm/store"],
  source: "mac-cleanup-sh pnpm")

rules << r(id: "src-mac-npm-cacache", name: "npm cacache",
  description: "npm cache clean --force location.",
  category: "package-cache", platform: MAC,
  paths: ["$HOME/.npm/_cacache", "$HOME/.npm/_logs"],
  source: "mac-cleanup-sh npm")

rules << r(id: "src-mac-gopath-mod", name: "Go module download cache",
  description: "go clean -modcache default GOPATH pkg/mod.",
  category: "package-cache", platform: MAC, risk: "medium", min_age: 14,
  paths: ["$HOME/go/pkg/mod", "$HOME/go/pkg/sumdb"],
  source: "mac-cleanup-sh go clean -modcache")

rules << r(id: "src-mac-kite-logs", name: "Kite autocomplete logs",
  description: "Kite editor plugin logs.",
  category: "logs", platform: MAC,
  paths: ["$HOME/.kite/logs"], source: "mac-cleanup-sh Kite")

rules << r(id: "src-mac-cacher-logs", name: "Cacher app logs",
  description: "Cacher snippet-app logs.",
  category: "logs", platform: MAC,
  paths: ["$HOME/.cacher/logs"], source: "mac-cleanup-sh Cacher")

rules << r(id: "src-mac-mail-logs", name: "Apple Mail container logs",
  description: "Mail.app diagnostic logs in the container. Not mailboxes.",
  category: "logs", platform: MAC,
  paths: ["$HOME/Library/Containers/com.apple.mail/Data/Library/Logs/Mail"],
  source: "mac-cleanup-sh Mail logs")

rules << r(id: "src-mac-diagnostic-reports", name: "User DiagnosticReports",
  description: "Crash reporter .crash/.ips in user Library. Not full sysdiagnose archives needed for support.",
  category: "logs", platform: MAC, min_age: 14,
  paths: ["$HOME/Library/Logs/DiagnosticReports"],
  source: "mac-cleanup-sh DiagnosticReports")

rules << r(id: "src-mac-ios-backups-report", name: "iOS device backups (report-only)",
  description: "Finder/iTunes MobileSync backups can be huge; never auto-delete.",
  category: "leftovers", platform: MAC, risk: "report-only", report_only: true, min_age: 0,
  paths: ["$HOME/Library/Application Support/MobileSync/Backup"],
  source: "mac-cleanup-sh iOS backups (flagged dangerous there)")

rules << r(id: "src-mac-itunes-ios-apps-report", name: "iTunes iOS app IPA cache (report-only)",
  description: "Downloaded IPAs under iTunes Media/Mobile Applications.",
  category: "leftovers", platform: MAC, risk: "report-only", report_only: true,
  paths: ["$HOME/Music/iTunes/iTunes Media/Mobile Applications"],
  source: "mac-cleanup-sh iOS Applications")

rules << r(id: "src-mac-jetbrains-logs", name: "JetBrains IDE logs",
  description: "IDEA/CLion/etc log directories. Not config or projects.",
  category: "logs", platform: MAC,
  paths: ["$HOME/Library/Logs/JetBrains"], source: "mac-cleanup-sh JetBrains")

rules << r(id: "src-mac-homebrew-cache-dir", name: "Homebrew bottle and cask cache",
  description: "brew --cache downloads. Formulae stay installed.",
  category: "package-cache", platform: MAC, min_age: 14,
  paths: [
    "$HOME/Library/Caches/Homebrew",
    "/opt/homebrew/var/homebrew/tmp",
    "/usr/local/var/homebrew/tmp"
  ], source: "mac-cleanup-sh brew cleanup / Homebrew docs")

# real Zoom bundle id (generated pack often uses fake us.zoom.Cache)
rules << r(id: "src-mac-zoom-xos", name: "Zoom client caches and logs",
  description: "us.zoom.xos caches, logs, and crashpad. Not recordings in Documents.",
  category: "app-cache", platform: MAC,
  paths: [
    "$HOME/Library/Caches/us.zoom.xos",
    "$HOME/Library/Logs/us.zoom.xos",
    "$HOME/Library/Application Support/zoom.us/data/WebviewCacheE4",
    "$HOME/Library/Application Support/zoom.us/data/CrashReports"
  ], source: "BleachBit zoom.xml + Zoom macOS bundle id")

# --- BleachBit Linux/Windows apps ---
[
  ["vlc", "VLC media player", "app-cache",
    { LIN => ["$HOME/.cache/vlc"], WIN => ["%APPDATA%/vlc"] }],
  ["transmission", "Transmission torrent", "app-cache",
    { LIN => ["$HOME/.cache/transmission", "$HOME/.config/transmission/resume"],
      MAC => ["$HOME/Library/Caches/org.m0k.transmission"],
      WIN => ["%LOCALAPPDATA%/transmission"] }],
  ["filezilla", "FileZilla", "app-cache",
    { LIN => ["$HOME/.cache/filezilla"] }],
  ["thunderbird", "Thunderbird cache", "browser-cache",
    { LIN => ["$HOME/.cache/thunderbird"],
      MAC => ["$HOME/Library/Caches/Thunderbird"],
      WIN => ["%APPDATA%/Thunderbird/Crash Reports"] }],
  ["libreoffice", "LibreOffice cache", "app-cache",
    { LIN => ["$HOME/.cache/libreoffice"],
      MAC => ["$HOME/Library/Caches/LibreOffice"] }],
  ["wine", "Wine cache", "wine-cache",
    { LIN => ["$HOME/.cache/wine", "$HOME/.cache/winetricks"] }],
  ["java", "Java deployment cache", "caches",
    { LIN => ["$HOME/.java/deployment/cache"],
      MAC => ["$HOME/Library/Caches/Java/cache", "$HOME/Library/Application Support/Oracle/Java/Deployment/cache"],
      WIN => ["%LOCALAPPDATA%/Sun/Java/Deployment/cache", "%USERPROFILE%/AppData/LocalLow/Sun/Java/Deployment/cache"] }],
  ["flash", "Adobe Flash Player cache leftover", "leftovers",
    { LIN => ["$HOME/.macromedia", "$HOME/.adobe"],
      MAC => ["$HOME/Library/Caches/Adobe/Flash Player"],
      WIN => ["%APPDATA%/Adobe/Flash Player", "%APPDATA%/Macromedia/Flash Player"] }]
].each do |id, name, cat, by_os|
  by_os.each do |plat, paths|
    next if plat == LIN && id == "wine" # linux.yaml already has linux-app-wine
    rules << r(id: "src-#{plat[0,3]}-bb-#{id}", name: "#{name} (#{plat})",
      description: "Regenerable cache/logs for #{name}. Not documents or prefs.",
      category: cat, platform: plat, paths: paths, source: "BleachBit cleaners/#{id}.xml")
  end
end

rules << r(id: "src-lin-bb-thumbnails", name: "XDG thumbnails (BleachBit)",
  description: "Freedesktop thumbnail cache. Regenerated by file managers.",
  category: "caches", platform: LIN, enabled: true, min_age: 14,
  paths: ["$HOME/.thumbnails", "$HOME/.cache/thumbnails", "$XDG_CACHE_HOME/thumbnails"],
  source: "BleachBit thumbnails.xml")

rules << r(id: "src-lin-bb-apt-partial", name: "APT partial downloads",
  description: "Incomplete apt package downloads under /var/cache/apt.",
  category: "package-cache", platform: LIN, risk: "report-only", report_only: true,
  paths: ["/var/cache/apt/archives/partial", "/var/cache/apt/archives"],
  source: "BleachBit apt.xml + Stacer APT cache")

rules << r(id: "src-lin-bb-yum-dnf", name: "YUM/DNF package caches",
  description: "dnf/yum metadata and package caches. Needs root to delete.",
  category: "package-cache", platform: LIN, risk: "report-only", report_only: true,
  paths: ["/var/cache/yum", "/var/cache/dnf"],
  source: "BleachBit yum.xml / Unix.py dnf")

rules << r(id: "src-lin-bb-pacman-pkg", name: "Pacman package cache",
  description: "Downloaded Arch packages. pacman -Sc recommended instead of blind delete of last copies.",
  category: "package-cache", platform: LIN, risk: "report-only", report_only: true,
  paths: ["/var/cache/pacman/pkg"],
  source: "BleachBit pacman.xml + Arch Wiki Clean the filesystem")

rules << r(id: "src-lin-bb-journal-user", name: "User systemd journal",
  description: "Per-user journal files. Not /var/log/journal (root).",
  category: "logs", platform: LIN, min_age: 14,
  paths: ["$HOME/.local/share/systemd/journal"],
  source: "BleachBit journald.xml / systemd user journals")

rules << r(id: "src-lin-bb-snap-user", name: "Snap user common cache",
  description: "Per-snap common cache dirs under ~/snap/*/common/.cache. Not snap revisions or ~/snap/*/current.",
  category: "caches", platform: LIN, risk: "medium",
  paths: ["$HOME/snap"], max_depth: 5,
  source: "BleachBit snap.xml")

rules << r(id: "src-lin-bb-chromium-sandboxes", name: "Chromium snap and Flatpak cache",
  description: "Chromium cache in snap/flatpak sandboxes. Not bookmarks.",
  category: "browser-cache", platform: LIN,
  paths: [
    "$HOME/snap/chromium/common/chromium",
    "$HOME/.var/app/org.chromium.Chromium/cache",
    "$HOME/.var/app/org.chromium.Chromium/config/chromium/Default/Cache",
    "$HOME/.var/app/com.google.Chrome/cache",
    "$HOME/.var/app/org.mozilla.firefox/cache"
  ], source: "BleachBit chromium.xml / google_chrome.xml sandbox paths")

rules << r(id: "src-lin-bb-zoom-home", name: "Zoom ~/.zoom cache",
  description: "Zoom Linux logs/cache. Not Documents/Zoom recordings.",
  category: "app-cache", platform: LIN,
  paths: ["$HOME/.zoom/logs", "$HOME/.zoom/data/WebviewCacheE4"],
  source: "BleachBit zoom.xml")

rules << r(id: "src-win-bb-zoom", name: "Zoom Windows cache and logs",
  description: "Zoom %AppData% cache/bin/logs. Not Documents\\Zoom recordings.",
  category: "app-cache", platform: WIN,
  paths: ["%APPDATA%/Zoom/data", "%APPDATA%/Zoom/logs", "%APPDATA%/Zoom/bin"],
  source: "BleachBit zoom.xml")

# --- Arch Wiki / Stacer ---
rules << r(id: "src-lin-arch-xdg-cache-report", name: "Entire ~/.cache (report-only)",
  description: "Arch Wiki notes ~/.cache can grow; report size only — apps store mixed data.",
  category: "caches", platform: LIN, risk: "report-only", report_only: true, min_age: 30,
  paths: ["$HOME/.cache", "$XDG_CACHE_HOME"],
  source: "Arch Wiki System maintenance § Old configuration files")

rules << r(id: "src-lin-stacer-crash", name: "Linux crash reports",
  description: "Apport/whoopsie crash dumps. Keep current Xorg/apt logs.",
  category: "logs", platform: LIN, min_age: 14,
  paths: ["/var/crash", "$HOME/.cache/apport"],
  source: "Stacer System Cleaner crash reports")

rules << r(id: "src-lin-var-tmp", name: "/var/tmp leftovers",
  description: "Persistent temp (survives reboot). Age-gated; some installers use it.",
  category: "temp", platform: LIN, risk: "medium", min_age: 14,
  paths: ["/var/tmp"], max_depth: 3,
  source: "FHS / Linux admin practice; Stacer temp")

# --- Winapp2 / CCleaner community ---
rules << r(id: "src-win-nvidia-shader", name: "NVIDIA shader / DX caches",
  description: "NVIDIA NV_Cache and DXCache. Games recompile shaders.",
  category: "shader-cache", platform: WIN, min_age: 14,
  paths: [
    "%LOCALAPPDATA%/NVIDIA/DXCache",
    "%LOCALAPPDATA%/NVIDIA/GLCache",
    "%LOCALAPPDATA%/NVIDIA Corporation/NV_Cache",
    "%LOCALAPPDATA%/NVIDIA Corporation/GfeSDK"
  ], source: "Winapp2.ini NVIDIA shader cache entries")

rules << r(id: "src-win-amd-shader", name: "AMD DirectX / OpenGL caches",
  description: "AMD DXCache/GLCache under LocalAppData.",
  category: "shader-cache", platform: WIN, min_age: 14,
  paths: ["%LOCALAPPDATA%/AMD/DxCache", "%LOCALAPPDATA%/AMD/GLCache", "%LOCALAPPDATA%/AMD/VkCache"],
  source: "Winapp2.ini AMD cache")

rules << r(id: "src-win-intel-shader", name: "Intel GPU shader caches",
  description: "Intel ShaderCache / DXCache.",
  category: "shader-cache", platform: WIN,
  paths: ["%LOCALAPPDATA%/Intel/ShaderCache", "%USERPROFILE%/AppData/LocalLow/Intel/ShaderCache"],
  source: "Winapp2.ini Intel shader")

rules << r(id: "src-win-edge-chromium-caches", name: "Edge Chromium disk caches",
  description: "Edge User Data Cache/Code Cache/GPUCache/Service Worker/CacheStorage. Not Favorites.",
  category: "browser-cache", platform: WIN, min_age: 3,
  paths: [
    "%LOCALAPPDATA%/Microsoft/Edge/User Data/Default/Cache",
    "%LOCALAPPDATA%/Microsoft/Edge/User Data/Default/Code Cache",
    "%LOCALAPPDATA%/Microsoft/Edge/User Data/Default/GPUCache",
    "%LOCALAPPDATA%/Microsoft/Edge/User Data/Default/Service Worker/CacheStorage",
    "%LOCALAPPDATA%/Microsoft/Edge/User Data/Default/DawnCache",
    "%LOCALAPPDATA%/Microsoft/Edge/User Data/ShaderCache"
  ], source: "Winapp2.ini Chromium Caches * / BleachBit microsoft_edge.xml")

rules << r(id: "src-win-chrome-full-caches", name: "Chrome Default Cache/GPU/Code/SW",
  description: "Chrome Default profile caches. Not Login Data.",
  category: "browser-cache", platform: WIN, min_age: 3,
  paths: [
    "%LOCALAPPDATA%/Google/Chrome/User Data/Default/Cache",
    "%LOCALAPPDATA%/Google/Chrome/User Data/Default/Code Cache",
    "%LOCALAPPDATA%/Google/Chrome/User Data/Default/GPUCache",
    "%LOCALAPPDATA%/Google/Chrome/User Data/Default/Service Worker/CacheStorage",
    "%LOCALAPPDATA%/Google/Chrome/User Data/ShaderCache",
    "%LOCALAPPDATA%/Google/Chrome/User Data/GrShaderCache"
  ], source: "Winapp2.ini / BleachBit google_chrome.xml")

rules << r(id: "src-win-firefox-cache2", name: "Firefox cache2 and startup cache",
  description: "cache2 and startupCache under Profiles. Not places.sqlite.",
  category: "browser-cache", platform: WIN, min_age: 3,
  paths: ["%LOCALAPPDATA%/Mozilla/Firefox/Profiles"],
  source: "BleachBit firefox.xml cache2")

rules << r(id: "src-win-defender-scan-history", name: "Windows Defender scan leftovers",
  description: "Defender Scans/Support logs. Quarantine is not included.",
  category: "logs", platform: WIN, risk: "report-only", report_only: true,
  paths: ["%PROGRAMDATA%/Microsoft/Windows Defender/Support", "%PROGRAMDATA%/Microsoft/Windows Defender/Scans/History"],
  source: "BleachBit windows_defender.xml")

rules << r(id: "src-win-prefetch-report", name: "Windows Prefetch (report-only)",
  description: "Prefetch can speed launches; report size only.",
  category: "system", platform: WIN, risk: "report-only", report_only: true,
  paths: ["%WINDIR%/Prefetch"],
  source: "CCleaner/Winapp2 system; treat as report-only")

rules << r(id: "src-win-recent-automaticdest", name: "Jump list AutomaticDestinations",
  description: "Explorer jump-list binaries. Shortcuts only.",
  category: "leftovers", platform: WIN, min_age: 14,
  paths: ["%APPDATA%/Microsoft/Windows/Recent/AutomaticDestinations", "%APPDATA%/Microsoft/Windows/Recent/CustomDestinations"],
  source: "Winapp2.ini Windows Explorer recent")

rules << r(id: "src-win-office-cache", name: "Microsoft Office file caches",
  description: "Office Document Cache / Office File Cache. Not documents.",
  category: "app-cache", platform: WIN, min_age: 14,
  paths: [
    "%LOCALAPPDATA%/Microsoft/Office/16.0/OfficeFileCache",
    "%LOCALAPPDATA%/Microsoft/Windows/INetCache/Content.Word",
    "%LOCALAPPDATA%/Microsoft/Office/UnsavedFiles"
  ], source: "BleachBit microsoft_office.xml / Winapp2 Office")

rules << r(id: "src-win-onedrive-logs", name: "OneDrive logs and cache",
  description: "OneDrive logs and Personal Vault leftovers cache. Not synced files.",
  category: "logs", platform: WIN,
  paths: ["%LOCALAPPDATA%/Microsoft/OneDrive/logs", "%LOCALAPPDATA%/Microsoft/OneDrive/cache"],
  source: "Winapp2.ini OneDrive")

rules << r(id: "src-win-spotify-data", name: "Spotify Data cache",
  description: "Spotify Data Storage cache of streamed tracks. Not playlists in local files you own.",
  category: "media-cache", platform: WIN, risk: "medium",
  paths: ["%LOCALAPPDATA%/Spotify/Storage", "%APPDATA%/Spotify/Data", "%APPDATA%/Spotify/Cache"],
  source: "Winapp2.ini Spotify")

rules << r(id: "src-win-discord-variants", name: "Discord Canary/PTB/PTB caches",
  description: "Electron Cache/Code Cache/GPUCache for Discord channels.",
  category: "app-cache", platform: WIN,
  paths: [
    "%APPDATA%/discordcanary/Cache", "%APPDATA%/discordcanary/Code Cache", "%APPDATA%/discordcanary/GPUCache",
    "%APPDATA%/discordptb/Cache", "%APPDATA%/discordptb/Code Cache", "%APPDATA%/discordptb/GPUCache"
  ], source: "Winapp2.ini Discord")

rules << r(id: "src-win-slack-service-worker", name: "Slack Service Worker and GPU",
  description: "Slack Electron SW cache and GPUCache.",
  category: "app-cache", platform: WIN,
  paths: ["%APPDATA%/Slack/Service Worker", "%APPDATA%/Slack/GPUCache", "%APPDATA%/Slack/DawnCache"],
  source: "Winapp2.ini Slack")

rules << r(id: "src-win-teams-new", name: "New Teams MSTeams cache",
  description: "Microsoft Teams (new) LocalCache. Not chat cloud data.",
  category: "app-cache", platform: WIN,
  paths: [
    "%LOCALAPPDATA%/Packages/MSTeams_8wekyb3d8bbwe/LocalCache",
    "%LOCALAPPDATA%/Microsoft/Teams"
  ], source: "Winapp2.ini Teams")

rules << r(id: "src-win-epic-launcher", name: "Epic Games Launcher webcache",
  description: "Epic Launcher webcache/Saved/webcache. Not game installs.",
  category: "game-cache", platform: WIN,
  paths: [
    "%LOCALAPPDATA%/EpicGamesLauncher/Saved/webcache",
    "%LOCALAPPDATA%/EpicGamesLauncher/Saved/webcache_4430",
    "%LOCALAPPDATA%/EpicGamesLauncher/Saved/Logs"
  ], source: "Winapp2.ini Epic Games")

rules << r(id: "src-win-steam-shader-depot", name: "Steam shaderdepot and htmlcache",
  description: "Steam htmlcache, appcache, shadercache. Not steamapps/common.",
  category: "game-cache", platform: WIN,
  paths: [
    "%PROGRAMFILES(X86)%/Steam/appcache",
    "%PROGRAMFILES(X86)%/Steam/depotcache",
    "%PROGRAMFILES(X86)%/Steam/steamapps/shadercache",
    "%LOCALAPPDATA%/Steam/htmlcache"
  ], source: "Winapp2.ini Steam + mac-cleanup Steam analog")

rules << r(id: "src-win-origin-ea", name: "EA App / Origin cache",
  description: "Origin/EA Desktop cache and logs. Not games.",
  category: "game-cache", platform: WIN,
  paths: [
    "%PROGRAMDATA%/Origin/Logs",
    "%PROGRAMDATA%/EA Desktop/Logs",
    "%LOCALAPPDATA%/Origin/Origin/cache",
    "%LOCALAPPDATA%/Electronic Arts/EA Desktop/cache"
  ], source: "Winapp2.ini Origin/EA")

rules << r(id: "src-win-ubisoft", name: "Ubisoft Connect cache",
  description: "Ubisoft Game Launcher cache/logs.",
  category: "game-cache", platform: WIN,
  paths: ["%LOCALAPPDATA%/Ubisoft Game Launcher/cache", "%LOCALAPPDATA%/Ubisoft Game Launcher/logs"],
  source: "Winapp2.ini Ubisoft")

rules << r(id: "src-win-battle-net", name: "Battle.net cache",
  description: "Battle.net Cache and Logs. Not game installs.",
  category: "game-cache", platform: WIN,
  paths: ["%PROGRAMDATA%/Battle.net/Setup", "%APPDATA%/Battle.net", "%LOCALAPPDATA%/Battle.net/Cache"],
  source: "Winapp2.ini Battle.net")

rules << r(id: "src-win-adobe-media", name: "Adobe Media Cache Windows",
  description: "Adobe Common Media Cache. Rebuilds peaks.",
  category: "media-cache", platform: WIN, risk: "medium",
  paths: ["%APPDATA%/Adobe/Common/Media Cache Files", "%APPDATA%/Adobe/Common/Media Cache"],
  source: "Winapp2.ini Adobe + mac-cleanup analog")

rules << r(id: "src-win-unity-gi-cache", name: "Unity GI and Library caches (user)",
  description: "Unity editor GI cache and Asset Store cache in AppData. Not project Assets.",
  category: "dev", platform: WIN, min_age: 14,
  paths: ["%LOCALAPPDATA%/Unity/cache", "%LOCALAPPDATA%/Unity/Editor/Cache", "%APPDATA%/Unity/Asset Store-5.x"],
  source: "Winapp2.ini Unity / Unity docs GI cache")

rules << r(id: "src-win-vs-componentcache", name: "Visual Studio ComponentModelCache",
  description: "VS MEF component cache. Rebuilds on next start.",
  category: "dev", platform: WIN,
  paths: [
    "%LOCALAPPDATA%/Microsoft/VisualStudio/17.0/ComponentModelCache",
    "%LOCALAPPDATA%/Microsoft/VisualStudio/16.0/ComponentModelCache"
  ], source: "Winapp2.ini Visual Studio")

rules << r(id: "src-win-pip-cache", name: "pip HTTP cache Windows",
  description: "pip download cache.",
  category: "package-cache", platform: WIN, min_age: 14,
  paths: ["%LOCALAPPDATA%/pip/Cache"],
  source: "pip docs + Winapp2")

rules << r(id: "src-win-npm-cache", name: "npm cache Windows",
  description: "npm _cacache on Windows.",
  category: "package-cache", platform: WIN, min_age: 14,
  paths: ["%APPDATA%/npm-cache", "%LOCALAPPDATA%/npm-cache"],
  source: "npm cache clean --force")

rules << r(id: "src-win-yarn-cache", name: "Yarn cache Windows",
  description: "Yarn cache folder.",
  category: "package-cache", platform: WIN,
  paths: ["%LOCALAPPDATA%/Yarn/Cache", "%LOCALAPPDATA%/Yarn/Berry/cache"],
  source: "yarn cache clean")

rules << r(id: "src-win-gradle-caches", name: "Gradle caches Windows",
  description: "%USERPROFILE%\\.gradle\\caches. Not settings.",
  category: "package-cache", platform: WIN, min_age: 14,
  paths: ["%USERPROFILE%/.gradle/caches", "%USERPROFILE%/.gradle/wrapper/dists", "%USERPROFILE%/.gradle/daemon"],
  source: "StackOverflow Gradle cache Windows path")

rules << r(id: "src-win-m2-repository", name: "Maven local repository (report-only)",
  description: ".m2/repository can be rebuilt but is large; report by default.",
  category: "package-cache", platform: WIN, risk: "report-only", report_only: true, min_age: 30,
  paths: ["%USERPROFILE%/.m2/repository"],
  source: "Maven docs / Gradle+Maven local")

rules << r(id: "src-win-chocolatey-lib-bad", name: "Chocolatey lib-bad and logs",
  description: "Failed Chocolatey installs and logs. Not lib of installed packages.",
  category: "package-cache", platform: WIN,
  paths: ["%PROGRAMDATA%/chocolatey/lib-bad", "%PROGRAMDATA%/chocolatey/logs"],
  source: "Chocolatey troubleshooting / Winapp2")

# --- extra Linux/macOS developer caches commonly listed on forums ---
[
  [MAC, "src-mac-bun-cache", "Bun install cache", ["$HOME/.bun/install/cache"], "Bun docs cache"],
  [LIN, "src-lin-bun-cache", "Bun install cache", ["$HOME/.bun/install/cache"], "Bun docs cache"],
  [WIN, "src-win-bun-cache", "Bun install cache", ["%USERPROFILE%/.bun/install/cache"], "Bun docs cache"],
  [MAC, "src-mac-deno-cache", "Deno remote cache", ["$HOME/Library/Caches/deno", "$HOME/.cache/deno"], "Deno DENO_DIR"],
  [LIN, "src-lin-deno-cache", "Deno remote cache", ["$HOME/.cache/deno"], "Deno DENO_DIR"],
  [WIN, "src-win-deno-cache", "Deno remote cache", ["%LOCALAPPDATA%/deno"], "Deno DENO_DIR"],
  [MAC, "src-mac-pdm-cache", "PDM cache", ["$HOME/Library/Caches/pdm", "$HOME/.cache/pdm"], "PDM cache dir"],
  [LIN, "src-lin-pdm-cache", "PDM cache", ["$HOME/.cache/pdm"], "PDM cache dir"],
  [MAC, "src-mac-conda-pkgs", "Conda pkgs cache", ["$HOME/Library/Caches/conda", "$HOME/.conda/pkgs"], "Conda pkgs"],
  [LIN, "src-lin-conda-pkgs", "Conda pkgs cache", ["$HOME/.conda/pkgs", "$HOME/miniconda3/pkgs", "$HOME/anaconda3/pkgs"], "Conda pkgs"],
  [WIN, "src-win-conda-pkgs", "Conda pkgs cache", ["%USERPROFILE%/.conda/pkgs", "%USERPROFILE%/miniconda3/pkgs", "%USERPROFILE%/anaconda3/pkgs"], "Conda pkgs"],
  [MAC, "src-mac-pixi-cache", "Pixi cache", ["$HOME/.cache/rattler", "$HOME/.pixi"], "Pixi/rattler cache"],
  [LIN, "src-lin-pixi-cache", "Pixi cache", ["$HOME/.cache/rattler", "$HOME/.pixi"], "Pixi/rattler cache"],
  [MAC, "src-mac-ccache", "ccache", ["$HOME/.ccache", "$HOME/Library/Caches/ccache"], "ccache dir"],
  [MAC, "src-mac-sccache", "sccache", ["$HOME/Library/Caches/Mozilla.sccache", "$HOME/.cache/sccache"], "sccache"],
  [LIN, "src-lin-sccache-extra", "sccache", ["$HOME/.cache/sccache"], "sccache"],
  [MAC, "src-mac-huggingface", "Hugging Face hub cache", ["$HOME/.cache/huggingface"], "HF_HOME / transformers cache"],
  [LIN, "src-lin-huggingface-hub", "Hugging Face hub cache", ["$HOME/.cache/huggingface"], "HF_HOME"],
  [WIN, "src-win-huggingface", "Hugging Face hub cache", ["%USERPROFILE%/.cache/huggingface"], "HF_HOME"],
  [MAC, "src-mac-torch", "PyTorch hub cache", ["$HOME/.cache/torch"], "TORCH_HOME"],
  [LIN, "src-lin-torch-hub", "PyTorch hub cache", ["$HOME/.cache/torch"], "TORCH_HOME"],
  [MAC, "src-mac-playwright", "Playwright browsers cache", ["$HOME/Library/Caches/ms-playwright"], "PLAYWRIGHT_BROWSERS_PATH"],
  [LIN, "src-lin-playwright-ms", "Playwright browsers cache", ["$HOME/.cache/ms-playwright"], "PLAYWRIGHT_BROWSERS_PATH"],
  [WIN, "src-win-playwright", "Playwright browsers cache", ["%USERPROFILE%/AppData/Local/ms-playwright"], "PLAYWRIGHT_BROWSERS_PATH"],
  [MAC, "src-mac-cypress", "Cypress binary cache", ["$HOME/Library/Caches/Cypress"], "Cypress cache"],
  [LIN, "src-lin-cypress-cache", "Cypress binary cache", ["$HOME/.cache/Cypress"], "Cypress cache"],
  [WIN, "src-win-cypress", "Cypress binary cache", ["%LOCALAPPDATA%/Cypress/Cache"], "Cypress cache"],
  [MAC, "src-mac-electron-builder", "electron-builder cache", ["$HOME/Library/Caches/electron-builder", "$HOME/Library/Caches/electron"], "electron-builder"],
  [LIN, "src-lin-electron-builder-cache", "electron-builder cache", ["$HOME/.cache/electron-builder", "$HOME/.cache/electron"], "electron-builder"],
  [MAC, "src-mac-turbo", "Turborepo cache", ["$HOME/Library/Caches/turbo", "$HOME/.turbo"], "turbo cache"],
  [LIN, "src-lin-turbo", "Turborepo cache", ["$HOME/.cache/turbo", "$HOME/.turbo"], "turbo cache"],
  [MAC, "src-mac-nx", "Nx cache", ["$HOME/.nx/cache"], "Nx cache"],
  [LIN, "src-lin-nx", "Nx cache", ["$HOME/.nx/cache"], "Nx cache"],
  [MAC, "src-mac-pre-commit", "pre-commit cache", ["$HOME/.cache/pre-commit"], "pre-commit"],
  [LIN, "src-lin-pre-commit-cache", "pre-commit cache", ["$HOME/.cache/pre-commit"], "pre-commit"],
  [MAC, "src-mac-helm", "Helm cache", ["$HOME/Library/Caches/helm", "$HOME/.cache/helm"], "helm cache"],
  [LIN, "src-lin-helm-cache", "Helm cache", ["$HOME/.cache/helm"], "helm"],
  [MAC, "src-mac-terraform", "Terraform plugin cache", ["$HOME/.terraform.d/plugin-cache"], "TF_PLUGIN_CACHE_DIR"],
  [LIN, "src-lin-terraform-plugins", "Terraform plugin cache", ["$HOME/.terraform.d/plugin-cache"], "TF_PLUGIN_CACHE_DIR"],
].each do |plat, id, name, paths, src|
  cat = %w[huggingface torch].any? { |k| id.include?(k) } ? "ml-cache" : "package-cache"
  cat = "dev" if %w[ccache sccache playwright cypress electron turbo nx pre-commit terraform].any? { |k| id.include?(k) }
  rules << r(id: id, name: name, description: "Regenerable tool cache. Config files are not targeted.",
    category: cat, platform: plat, paths: paths, min_age: 14, source: src)
end

# macOS Library locations Apple / forums call regenerable
rules << r(id: "src-mac-saved-state-user", name: "Saved Application State",
  description: "Window restoration blobs. Apps recreate them. Not documents.",
  category: "leftovers", platform: MAC, min_age: 14,
  paths: ["$HOME/Library/Saved Application State"],
  source: "Apple discussions / macOS Library cleanup guides")

rules << r(id: "src-mac-httpstorages", name: "URLSession HTTPStorages",
  description: "Per-app HTTP cookie/cache stores under HTTPStorages. Mixed with cookies — report-only.",
  category: "caches", platform: MAC, risk: "report-only", report_only: true,
  paths: ["$HOME/Library/HTTPStorages"],
  source: "Apple URLSession disk cache layout")

rules << r(id: "src-mac-quicklook-thumbnails", name: "Quick Look thumbnails cache",
  description: "com.apple.QuickLook.thumbnailcache / qlmanage cache.",
  category: "caches", platform: MAC, enabled: true, min_age: 7,
  paths: [
    "$HOME/Library/Caches/com.apple.QuickLook.thumbnailcache",
    "$HOME/Library/Caches/CloudKit/com.apple.quicklook.QuickLookUIService"
  ], source: "macOS Quick Look cache; community cleanup scripts")

rules << r(id: "src-mac-font-registrations", name: "User font cache",
  description: "ATS/font caches. Regenerated. Not ~/Library/Fonts files.",
  category: "caches", platform: MAC,
  paths: ["$HOME/Library/Caches/com.apple.FontRegistry", "$HOME/Library/Caches/com.apple.ATS"],
  source: "Apple font cache troubleshooting")

rules << r(id: "src-mac-maps-cache", name: "Maps tile cache",
  description: "Maps app tile caches. Offline areas may need re-download.",
  category: "caches", platform: MAC, risk: "medium",
  paths: ["$HOME/Library/Caches/com.apple.Maps", "$HOME/Library/Containers/com.apple.Maps/Data/Library/Caches"],
  source: "Apple Maps cache community reports")

rules << r(id: "src-mac-podcasts-cache", name: "Podcasts media cache",
  description: "Podcasts app cached episodes in group container cache. Not subscribed show list.",
  category: "media-cache", platform: MAC, risk: "medium",
  paths: ["$HOME/Library/Group Containers/243LU875E5.groups.com.apple.podcasts/Library/Cache"],
  source: "macOS Podcasts group container cache")

rules << r(id: "src-mac-tv-cache", name: "TV app asset cache",
  description: "TV/iTunes video asset cache.",
  category: "media-cache", platform: MAC, risk: "medium",
  paths: ["$HOME/Library/Caches/com.apple.TV", "$HOME/Library/Caches/com.apple.AppleTV"],
  source: "Apple TV cache threads")

rules << r(id: "src-mac-music-cache", name: "Music app artwork/stream cache",
  description: "Music.app caches. Not Apple Music downloads library.",
  category: "media-cache", platform: MAC, risk: "report-only", report_only: true,
  paths: ["$HOME/Library/Caches/com.apple.Music", "$HOME/Library/Caches/com.apple.AMPLibraryAgent"],
  source: "Apple Music cache forum reports")

rules << r(id: "src-mac-xcode-device-support", name: "Xcode iOS DeviceSupport (report-only)",
  description: "Symbols for each iOS version; large but useful for debugging. Report only.",
  category: "dev", platform: MAC, risk: "report-only", report_only: true, min_age: 60,
  paths: ["$HOME/Library/Developer/Xcode/iOS DeviceSupport", "$HOME/Library/Developer/Xcode/watchOS DeviceSupport"],
  source: "Xcode DeviceSupport cleanup articles")

rules << r(id: "src-mac-simulator-caches", name: "CoreSimulator Caches",
  description: "Simulator Caches directory. Not named device data folders.",
  category: "dev", platform: MAC, risk: "medium", min_age: 14,
  paths: ["$HOME/Library/Developer/CoreSimulator/Caches"],
  source: "Xcode simulator cache / mac-cleanup simctl notes")

# Linux extra apps from BleachBit list
[
  ["audacious", ["$HOME/.cache/audacious"]],
  ["rhythmbox", ["$HOME/.cache/rhythmbox"]],
  ["sqlite3", ["$HOME/.sqlite_history"]],
].each do |id, paths|
  cat = id == "sqlite3" ? "leftovers" : "app-cache"
  rules << r(id: "src-lin-bb-#{id}", name: "Linux #{id}",
    description: "BleachBit cleaner leftover for #{id}. Not user media libraries.",
    category: cat, platform: LIN, paths: paths, source: "BleachBit #{id}.xml")
end

existing = load_ids
kept = rules.reject { |h| existing.include?(h["id"]) }
dupes = rules.size - kept.size

doc = {
  "pack" => "community-sources",
  "description" => "Curated paths from BleachBit, mac-cleanup-sh, Winapp2.ini, Arch Wiki, Stacer, and common package-manager docs. #{kept.size} unique rules (skipped #{dupes} existing ids).",
  "rules" => kept
}

File.write(OUT, YAML.dump(doc))
puts "Wrote #{kept.size} rules to #{OUT} (skipped #{dupes} colliding ids)"
