#!/usr/bin/env ruby
# frozen_string_literal: true

# Generate rules/platforms/{windows,linux}.yaml — OS-specific junk targets.
# Loader already walks rules/ recursively; ids must stay unique vs categories/.

require "yaml"
require "fileutils"

ROOT = File.expand_path("..", __dir__)
OUT = File.join(ROOT, "rules", "platforms")

def rule(id:, name:, description:, category:, platform:, paths:,
         risk: "low", enabled: false, min_age: 7, max_depth: 6,
         delete_mode: "contents", report_only: false, exclude: nil, include: nil)
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

def write_pack(name, description, rules)
  FileUtils.mkdir_p(OUT)
  path = File.join(OUT, "#{name}.yaml")
  doc = {
    "pack" => name,
    "description" => description,
    "rules" => rules
  }
  File.write(path, YAML.dump(doc))
  puts "wrote #{path} (#{rules.size} rules)"
end

# --- Windows -----------------------------------------------------------------

WIN_BROWSERS = [
  ["chrome", "%LOCALAPPDATA%/Google/Chrome/User Data/Default/Cache", "%LOCALAPPDATA%/Google/Chrome/User Data/Default/Code Cache", "%LOCALAPPDATA%/Google/Chrome/User Data/Default/GPUCache"],
  ["chrome-canary", "%LOCALAPPDATA%/Google/Chrome SxS/User Data/Default/Cache", "%LOCALAPPDATA%/Google/Chrome SxS/User Data/Default/Code Cache"],
  ["edge", "%LOCALAPPDATA%/Microsoft/Edge/User Data/Default/Cache", "%LOCALAPPDATA%/Microsoft/Edge/User Data/Default/Code Cache", "%LOCALAPPDATA%/Microsoft/Edge/User Data/Default/GPUCache"],
  ["brave", "%LOCALAPPDATA%/BraveSoftware/Brave-Browser/User Data/Default/Cache", "%LOCALAPPDATA%/BraveSoftware/Brave-Browser/User Data/Default/Code Cache"],
  ["firefox", "%LOCALAPPDATA%/Mozilla/Firefox/Profiles", "%APPDATA%/Mozilla/Firefox/Crash Reports"],
  ["opera", "%APPDATA%/Opera Software/Opera Stable/Cache", "%LOCALAPPDATA%/Opera Software/Opera Stable/Cache"],
  ["vivaldi", "%LOCALAPPDATA%/Vivaldi/User Data/Default/Cache", "%LOCALAPPDATA%/Vivaldi/User Data/Default/Code Cache"],
  ["tor", "%LOCALAPPDATA%/Tor Browser/Browser/TorBrowser/Data/Browser/Caches"],
  ["yandex", "%LOCALAPPDATA%/Yandex/YandexBrowser/User Data/Default/Cache"],
]

WIN_APPS = [
  ["discord", "%APPDATA%/discord/Cache", "%APPDATA%/discord/Code Cache", "%APPDATA%/discord/GPUCache"],
  ["slack", "%APPDATA%/Slack/Cache", "%APPDATA%/Slack/Code Cache", "%APPDATA%/Slack/GPUCache"],
  ["teams", "%APPDATA%/Microsoft/Teams/Cache", "%APPDATA%/Microsoft/Teams/blob_storage", "%APPDATA%/Microsoft/Teams/GPUCache"],
  ["zoom", "%APPDATA%/Zoom/logs", "%APPDATA%/Zoom/data"],
  ["spotify", "%LOCALAPPDATA%/Spotify/Storage", "%LOCALAPPDATA%/Spotify/Data"],
  ["steam", "%LOCALAPPDATA%/Steam/htmlcache", "%PROGRAMFILES(X86)%/Steam/appcache", "%PROGRAMFILES(X86)%/Steam/logs"],
  ["epic", "%LOCALAPPDATA%/EpicGamesLauncher/Saved/webcache", "%LOCALAPPDATA%/EpicGamesLauncher/Saved/Logs"],
  ["vscode", "%APPDATA%/Code/Cache", "%APPDATA%/Code/CachedData", "%APPDATA%/Code/GPUCache", "%APPDATA%/Code/logs"],
  ["cursor", "%APPDATA%/Cursor/Cache", "%APPDATA%/Cursor/CachedData", "%APPDATA%/Cursor/GPUCache", "%APPDATA%/Cursor/logs"],
  ["notion", "%APPDATA%/Notion/Cache", "%APPDATA%/Notion/Code Cache"],
  ["figma", "%APPDATA%/Figma/Desktop/Cache", "%APPDATA%/Figma/Desktop/Code Cache"],
  ["obsidian", "%APPDATA%/obsidian/Cache", "%APPDATA%/obsidian/Code Cache"],
  ["postman", "%APPDATA%/Postman/Cache", "%APPDATA%/Postman/logs"],
  ["docker", "%LOCALAPPDATA%/Docker/log", "%APPDATA%/Docker Desktop"],
  ["dropbox", "%LOCALAPPDATA%/Dropbox/logs", "%LOCALAPPDATA%/Dropbox/Crashpad"],
  ["onedrive", "%LOCALAPPDATA%/Microsoft/OneDrive/logs", "%LOCALAPPDATA%/Microsoft/OneDrive/setup/logs"],
  ["adobe-cc", "%LOCALAPPDATA%/Adobe/OOBE", "%APPDATA%/Adobe/Adobe Photoshop 2024/AutoRecover"],
  ["telegram", "%APPDATA%/Telegram Desktop/tdata/user_data", "%APPDATA%/Telegram Desktop/log.txt"],
  ["whatsapp", "%LOCALAPPDATA%/Packages/5319275A.WhatsAppDesktop_cv1g1gvanyjgm/LocalCache"],
  ["1password", "%LOCALAPPDATA%/1Password/logs"],
  ["nvidia", "%LOCALAPPDATA%/NVIDIA/DXCache", "%LOCALAPPDATA%/NVIDIA/GLCache", "%LOCALAPPDATA%/NVIDIA Corporation/NVIDIA App/CefCache"],
  ["amd", "%LOCALAPPDATA%/AMD/DxCache", "%LOCALAPPDATA%/AMD/Dx9Cache"],
]

WIN_PKG = [
  ["npm", "%APPDATA%/npm-cache", "%LOCALAPPDATA%/npm-cache"],
  ["yarn", "%LOCALAPPDATA%/Yarn/Cache", "%APPDATA%/Yarn/Cache"],
  ["pnpm", "%LOCALAPPDATA%/pnpm-store", "%LOCALAPPDATA%/pnpm/store"],
  ["pip", "%LOCALAPPDATA%/pip/Cache"],
  ["nuget", "%USERPROFILE%/.nuget/packages", "%LOCALAPPDATA%/NuGet/v3-cache"],
  ["cargo", "%USERPROFILE%/.cargo/registry/cache", "%USERPROFILE%/.cargo/git/db"],
  ["gradle", "%USERPROFILE%/.gradle/caches", "%USERPROFILE%/.gradle/wrapper/dists"],
  ["maven", "%USERPROFILE%/.m2/repository"],
  ["composer", "%APPDATA%/Composer/cache", "%LOCALAPPDATA%/Composer"],
  ["chocolatey", "%PROGRAMDATA%/chocolatey/lib", "%PROGRAMDATA%/chocolatey/logs"],
  ["scoop", "%USERPROFILE%/scoop/cache"],
  ["winget", "%LOCALAPPDATA%/Microsoft/WinGet/Packages", "%LOCALAPPDATA%/Microsoft/WinGet/Downloads"],
  ["dotnet", "%LOCALAPPDATA%/Temp/MSBuildTemp", "%USERPROFILE%/.dotnet/TelemetryStorageService"],
  ["go", "%LOCALAPPDATA%/go-build", "%USERPROFILE%/go/pkg/mod/cache"],
]

def windows_rules
  rules = []

  rules << rule(
    id: "win-temp-user",
    name: "User TEMP",
    description: "User TEMP / LocalAppData\\Temp (age-filtered).",
    category: "temp",
    platform: "windows",
    paths: ["%TEMP%", "%TMP%", "%LOCALAPPDATA%/Temp"],
    enabled: true,
    min_age: 2,
    max_depth: 6
  )
  rules << rule(
    id: "win-inet-cache",
    name: "Windows INetCache",
    description: "IE/Edge legacy Internet cache.",
    category: "browser-cache",
    platform: "windows",
    paths: ["%LOCALAPPDATA%/Microsoft/Windows/INetCache"],
    enabled: true,
    min_age: 3
  )
  rules << rule(
    id: "win-explorer-thumbcache",
    name: "Explorer thumbcache",
    description: "Explorer thumbnail / icon caches under LocalAppData.",
    category: "caches",
    platform: "windows",
    paths: ["%LOCALAPPDATA%/Microsoft/Windows/Explorer"],
    include: ["thumbcache_*.db", "iconcache_*.db", "*.db"],
    enabled: true,
    min_age: 7,
    max_depth: 2
  )
  rules << rule(
    id: "win-recent-items",
    name: "Recent items shortcuts",
    description: "Recent Documents / CustomDestinations (shortcuts only).",
    category: "leftovers",
    platform: "windows",
    paths: [
      "%APPDATA%/Microsoft/Windows/Recent",
      "%APPDATA%/Microsoft/Windows/Recent/AutomaticDestinations",
      "%APPDATA%/Microsoft/Windows/Recent/CustomDestinations"
    ],
    min_age: 30,
    max_depth: 3
  )
  rules << rule(
    id: "win-crash-dumps",
    name: "User crash dumps",
    description: "LocalAppData CrashDumps.",
    category: "logs",
    platform: "windows",
    paths: ["%LOCALAPPDATA%/CrashDumps"],
    enabled: true,
    min_age: 14
  )
  rules << rule(
    id: "win-wer-report",
    name: "Windows Error Reporting queue (report)",
    description: "WER ReportQueue — report-only.",
    category: "logs",
    platform: "windows",
    paths: ["%LOCALAPPDATA%/Microsoft/Windows/WER/ReportQueue", "%PROGRAMDATA%/Microsoft/Windows/WER/ReportQueue"],
    risk: "report-only",
    report_only: true,
    min_age: 14
  )
  rules << rule(
    id: "win-delivery-optimization",
    name: "Delivery Optimization cache",
    description: "Windows Update Delivery Optimization cache.",
    category: "caches",
    platform: "windows",
    paths: ["%WINDIR%/SoftwareDistribution/DeliveryOptimization/Cache", "%LOCALAPPDATA%/Microsoft/Windows/DeliveryOptimization"],
    risk: "medium",
    min_age: 14
  )
  rules << rule(
    id: "win-software-distribution-download",
    name: "Windows Update Download folder",
    description: "SoftwareDistribution\\Download (may force re-download of updates).",
    category: "caches",
    platform: "windows",
    paths: ["%WINDIR%/SoftwareDistribution/Download"],
    risk: "medium",
    min_age: 14
  )
  rules << rule(
    id: "win-prefetch-report",
    name: "Prefetch (report-only)",
    description: "Prefetch can affect launch analysis; never auto-delete.",
    category: "system",
    platform: "windows",
    paths: ["%WINDIR%/Prefetch", "C:/Windows/Prefetch"],
    risk: "report-only",
    report_only: true,
    min_age: 30,
    max_depth: 1
  )
  rules << rule(
    id: "win-windows-old-report",
    name: "Windows.old (report-only)",
    description: "Previous Windows install; use Storage Settings / Disk Cleanup.",
    category: "system",
    platform: "windows",
    paths: ["C:/Windows.old", "%SystemDrive%/Windows.old"],
    risk: "report-only",
    report_only: true,
    delete_mode: "path",
    min_age: 0,
    max_depth: 1
  )
  rules << rule(
    id: "win-recycle-bin-report",
    name: "Recycle Bin (report-only)",
    description: "Does not empty Recycle Bin; awareness only.",
    category: "leftovers",
    platform: "windows",
    paths: ["C:/$Recycle.Bin", "%SystemDrive%/$Recycle.Bin"],
    risk: "report-only",
    report_only: true,
    min_age: 0,
    max_depth: 2
  )
  rules << rule(
    id: "win-font-cache",
    name: "Font cache",
    description: "Windows FontCache service files.",
    category: "caches",
    platform: "windows",
    paths: ["%LOCALAPPDATA%/Microsoft/Windows/Fonts", "%WINDIR%/ServiceProfiles/LocalService/AppData/Local/FontCache"],
    min_age: 30
  )
  rules << rule(
    id: "win-directx-shader",
    name: "DirectX shader cache",
    description: "D3DSCache / DirectX shader caches.",
    category: "shader-cache",
    platform: "windows",
    paths: ["%LOCALAPPDATA%/D3DSCache", "%LOCALAPPDATA%/Microsoft/DirectX Shader Cache"],
    enabled: true,
    min_age: 14
  )
  rules << rule(
    id: "win-temp-internet-files",
    name: "Temporary Internet Files",
    description: "Legacy Temporary Internet Files folder.",
    category: "browser-cache",
    platform: "windows",
    paths: ["%LOCALAPPDATA%/Microsoft/Windows/Temporary Internet Files", "%USERPROFILE%/Local Settings/Temporary Internet Files"],
    min_age: 7
  )

  WIN_BROWSERS.each do |slug, *paths|
    rules << rule(
      id: "win-browser-#{slug}",
      name: "Browser cache #{slug}",
      description: "Windows browser cache paths for #{slug}.",
      category: "browser-cache",
      platform: "windows",
      paths: paths,
      enabled: %w[chrome edge].include?(slug),
      min_age: 3,
      max_depth: 8
    )
  end

  WIN_APPS.each do |slug, *paths|
    cat = case slug
          when "steam", "epic" then "game-cache"
          when "vscode", "cursor", "postman", "docker" then "dev"
          when /log/ then "logs"
          else "app-cache"
          end
    # split logs vs cache heuristically by path name
    cache_paths = paths.reject { |p| p.downcase.include?("log") }
    log_paths = paths.select { |p| p.downcase.include?("log") }
    unless cache_paths.empty?
      rules << rule(
        id: "win-app-#{slug}",
        name: "#{slug} app caches",
        description: "Windows app caches for #{slug}.",
        category: cat,
        platform: "windows",
        paths: cache_paths,
        min_age: 7,
        max_depth: 6
      )
    end
    unless log_paths.empty?
      rules << rule(
        id: "win-app-logs-#{slug}",
        name: "#{slug} logs",
        description: "Windows app logs for #{slug}.",
        category: "logs",
        platform: "windows",
        paths: log_paths,
        min_age: 14,
        max_depth: 6
      )
    end
  end

  WIN_PKG.each do |slug, *paths|
    rules << rule(
      id: "win-pkg-#{slug}",
      name: "#{slug} package cache",
      description: "Windows package/tooling cache for #{slug}.",
      category: "package-cache",
      platform: "windows",
      paths: paths,
      enabled: %w[npm pip nuget].include?(slug),
      min_age: 14,
      max_depth: 8
    )
  end

  # More Electron / LocalAppData caches
  %w[
    Discord Slack Spotify Notion Figma Obsidian Postman Insomnia Telegram
    Signal Element WhatsApp Zoom Teams Skype GitHubDesktop Sourcetree
    Atom Hyper Termius PowerToys Everything ShareX Greenshot
  ].each do |name|
    s = name.downcase
    rules << rule(
      id: "win-electron-#{s}",
      name: "#{name} Electron caches",
      description: "AppData Electron Cache/Code Cache/GPUCache for #{name}.",
      category: "app-cache",
      platform: "windows",
      paths: [
        "%APPDATA%/#{name}/Cache",
        "%APPDATA%/#{name}/Code Cache",
        "%APPDATA%/#{name}/GPUCache",
        "%APPDATA%/#{name}/CachedData",
        "%LOCALAPPDATA%/#{name}/Cache"
      ],
      min_age: 7
    )
  end

  rules
end

# --- Linux -------------------------------------------------------------------

LINUX_DEV = [
  [".cache/pip", "pip", "package-cache"],
  [".cache/uv", "uv", "package-cache"],
  [".cache/yarn", "yarn", "package-cache"],
  [".cache/pnpm", "pnpm", "package-cache"],
  [".cache/npm", "npm", "package-cache"],
  [".npm/_cacache", "npm-cacache", "package-cache"],
  [".npm/_logs", "npm-logs", "logs"],
  [".cache/ms-playwright", "playwright", "package-cache"],
  [".cache/puppeteer", "puppeteer", "package-cache"],
  [".cache/Cypress", "cypress", "package-cache"],
  [".cache/electron", "electron", "package-cache"],
  [".cache/electron-builder", "electron-builder", "package-cache"],
  [".cache/huggingface", "huggingface", "ml-cache"],
  [".cache/torch", "torch", "ml-cache"],
  [".cache/go-build", "go-build", "dev"],
  [".cache/ccache", "ccache", "dev"],
  [".cache/sccache", "sccache", "dev"],
  [".cache/clangd", "clangd", "dev"],
  [".cache/pre-commit", "pre-commit", "dev"],
  [".cache/JetBrains", "jetbrains", "dev"],
  [".cache/mozilla", "mozilla", "browser-cache"],
  [".cache/google-chrome", "chrome", "browser-cache"],
  [".cache/chromium", "chromium", "browser-cache"],
  [".cache/BraveSoftware", "brave", "browser-cache"],
  [".cache/thumbnails", "thumbnails", "caches"],
  [".cache/mesa_shader_cache", "mesa", "shader-cache"],
  [".cache/nvidia", "nvidia", "shader-cache"],
  [".cache/fontconfig", "fontconfig", "caches"],
  [".cache/tracker3", "tracker3", "caches"],
  [".cache/evolution", "evolution", "app-cache"],
  [".cache/flatpak", "flatpak", "package-cache"],
  [".cache/snap", "snapd-user", "package-cache"],
  [".local/share/Trash/files", "trash-files", "leftovers"],
  [".local/share/Trash/info", "trash-info", "leftovers"],
  [".local/share/xorg", "xorg", "logs"],
  [".cargo/registry/cache", "cargo-registry", "package-cache"],
  [".cargo/git/db", "cargo-git", "package-cache"],
  [".rustup/tmp", "rustup-tmp", "temp"],
  [".rustup/downloads", "rustup-dl", "package-cache"],
  [".gradle/caches", "gradle", "package-cache"],
  [".m2/repository", "maven", "package-cache"],
  [".composer/cache", "composer", "package-cache"],
  [".cache/composer", "composer-xdg", "package-cache"],
  [".cache/helm", "helm", "dev"],
  [".cache/terraform", "terraform", "dev"],
  [".docker", "docker-home", "leftovers"],
  [".local/share/containers", "podman", "vm-disk"],
  [".vagrant.d/boxes", "vagrant", "vm-disk"],
  [".minikube", "minikube", "vm-cache"],
  [".kube/cache", "kubectl", "dev"],
]

LINUX_APPS = [
  ["spotify", "$HOME/.cache/spotify"],
  ["discord", "$HOME/.config/discord/Cache", "$HOME/.config/discord/Code Cache", "$HOME/.config/discord/GPUCache"],
  ["slack", "$HOME/.config/Slack/Cache", "$HOME/.config/Slack/Code Cache"],
  ["teams", "$HOME/.config/Microsoft/Microsoft Teams/Cache"],
  ["zoom", "$HOME/.zoom/logs", "$HOME/.zoom/data"],
  ["steam", "$HOME/.steam/steam/appcache", "$HOME/.local/share/Steam/appcache", "$HOME/.local/share/Steam/logs"],
  ["vscode", "$HOME/.config/Code/Cache", "$HOME/.config/Code/CachedData", "$HOME/.config/Code/logs"],
  ["cursor", "$HOME/.config/Cursor/Cache", "$HOME/.config/Cursor/CachedData", "$HOME/.config/Cursor/logs"],
  ["obsidian", "$HOME/.config/obsidian/Cache", "$HOME/.config/obsidian/Code Cache"],
  ["firefox", "$HOME/.mozilla/firefox"],
  ["chrome", "$HOME/.config/google-chrome/Default/Cache", "$HOME/.config/google-chrome/Default/Code Cache"],
  ["chromium", "$HOME/.config/chromium/Default/Cache", "$HOME/.config/chromium/Default/Code Cache"],
  ["brave", "$HOME/.config/BraveSoftware/Brave-Browser/Default/Cache"],
  ["thunderbird", "$HOME/.thunderbird", "$HOME/.cache/thunderbird"],
  ["libreoffice", "$HOME/.cache/libreoffice"],
  ["gimp", "$HOME/.cache/gimp"],
  ["inkscape", "$HOME/.cache/inkscape"],
  ["blender", "$HOME/.cache/blender"],
  ["lutris", "$HOME/.cache/lutris", "$HOME/.local/share/lutris/runners"],
  ["wine", "$HOME/.cache/wine", "$HOME/.cache/winetricks"],
  ["bottles", "$HOME/.local/share/bottles/temp", "$HOME/.cache/bottles"],
]

def linux_rules
  rules = []

  rules << rule(
    id: "linux-xdg-cache-safe",
    name: "XDG cache (selected)",
    description: "Common safe subfolders under ~/.cache (not entire tree).",
    category: "caches",
    platform: "linux",
    paths: [
      "$HOME/.cache/thumbnails",
      "$HOME/.cache/fontconfig",
      "$HOME/.cache/mesa_shader_cache",
      "$XDG_CACHE_HOME/thumbnails",
      "$XDG_CACHE_HOME/fontconfig"
    ],
    enabled: true,
    min_age: 3,
    max_depth: 5
  )
  rules << rule(
    id: "linux-thumbnails",
    name: "Thumbnail cache",
    description: "Freedesktop thumbnail cache.",
    category: "caches",
    platform: "linux",
    paths: ["$HOME/.cache/thumbnails", "$HOME/.thumbnails"],
    enabled: true,
    min_age: 7
  )
  rules << rule(
    id: "linux-tmp",
    name: "User temp",
    description: "TMPDIR aged files.",
    category: "temp",
    platform: "linux",
    paths: ["$TMPDIR", "$TEMP", "/tmp"],
    exclude: ["systemd-private-*", "snap-*", ".X*-lock"],
    enabled: true,
    min_age: 2,
    max_depth: 3
  )
  rules << rule(
    id: "linux-trash",
    name: "User Trash files",
    description: "Freedesktop Trash files/info.",
    category: "leftovers",
    platform: "linux",
    paths: ["$HOME/.local/share/Trash/files", "$HOME/.local/share/Trash/info"],
    enabled: true,
    min_age: 14
  )
  rules << rule(
    id: "linux-user-logs",
    name: "User log files",
    description: "Common user-level log directories.",
    category: "logs",
    platform: "linux",
    paths: ["$HOME/.local/share/xorg", "$HOME/.local/state"],
    include: ["**/*.log", "**/*.old", "**/*.log.*"],
    enabled: true,
    min_age: 14
  )
  rules << rule(
    id: "linux-journal-report",
    name: "systemd journal (report-only)",
    description: "Does not vacuum journald; awareness only.",
    category: "logs",
    platform: "linux",
    paths: ["/var/log/journal"],
    risk: "report-only",
    report_only: true,
    delete_mode: "path",
    min_age: 0,
    max_depth: 1
  )
  rules << rule(
    id: "linux-var-log-report",
    name: "/var/log (report-only)",
    description: "System logs — report only; needs root to clean.",
    category: "logs",
    platform: "linux",
    paths: ["/var/log"],
    risk: "report-only",
    report_only: true,
    min_age: 30,
    max_depth: 2
  )
  rules << rule(
    id: "linux-apt-archives",
    name: "APT package archives",
    description: "Downloaded .deb archives (apt clean).",
    category: "package-cache",
    platform: "linux",
    paths: ["/var/cache/apt/archives"],
    risk: "medium",
    min_age: 7,
    max_depth: 2
  )
  rules << rule(
    id: "linux-pacman-cache",
    name: "pacman package cache",
    description: "Arch pacman package cache.",
    category: "package-cache",
    platform: "linux",
    paths: ["/var/cache/pacman/pkg"],
    risk: "medium",
    min_age: 14
  )
  rules << rule(
    id: "linux-dnf-cache",
    name: "DNF/YUM cache",
    description: "Fedora/RHEL package manager caches.",
    category: "package-cache",
    platform: "linux",
    paths: ["/var/cache/dnf", "/var/cache/yum"],
    risk: "medium",
    min_age: 14
  )
  rules << rule(
    id: "linux-snap-cache",
    name: "Snap user revisions leftover",
    description: "Snap user common/cache folders.",
    category: "package-cache",
    platform: "linux",
    paths: ["$HOME/snap"],
    include: ["**/common/.cache/**", "**/current/.cache/**"],
    min_age: 14,
    max_depth: 8
  )
  rules << rule(
    id: "linux-flatpak-cache",
    name: "Flatpak user cache",
    description: "Flatpak app caches under ~/.var/app.",
    category: "app-cache",
    platform: "linux",
    paths: ["$HOME/.var/app"],
    include: ["**/cache/**", "**/Cache/**"],
    min_age: 14,
    max_depth: 10
  )

  LINUX_DEV.each do |rel, slug, cat|
    rules << rule(
      id: "linux-dev-#{slug}",
      name: "Linux #{slug}",
      description: "Developer/tool cache at ~/#{rel}.",
      category: cat,
      platform: "linux",
      paths: ["$HOME/#{rel}", "$XDG_CACHE_HOME/#{rel.sub(%r{^\.cache/}, '')}"].uniq,
      enabled: %w[thumbnails pip yarn npm chrome].include?(slug),
      min_age: 7,
      max_depth: 8
    )
  end

  LINUX_APPS.each do |slug, *paths|
    cat = case slug
          when "steam", "lutris", "wine", "bottles" then "game-cache"
          when "firefox", "chrome", "chromium", "brave" then "browser-cache"
          when "vscode", "cursor" then "dev"
          else "app-cache"
          end
    rules << rule(
      id: "linux-app-#{slug}",
      name: "Linux #{slug}",
      description: "Linux app paths for #{slug}.",
      category: cat,
      platform: "linux",
      paths: paths,
      min_age: 7,
      max_depth: 8
    )
  end

  # Distro package leftovers under home
  %w[rpmbuild .ccache .local/share/flatpak .local/share/Trash].each do |rel|
    rules << rule(
      id: "linux-home-#{rel.gsub(%r{[./]}, '-')}",
      name: "Home #{rel}",
      description: "Home leftover #{rel}.",
      category: (rel.include?("Trash") ? "leftovers" : "dev"),
      platform: "linux",
      paths: ["$HOME/#{rel}"],
      min_age: 14
    )
  end

  rules
end

# --- main --------------------------------------------------------------------

existing = {}
Dir[File.join(ROOT, "rules", "**", "*.yaml")].each do |f|
  next if f.include?("/platforms/")
  data = YAML.load_file(f) || {}
  (data["rules"] || []).each { |r| existing[r["id"]] = true }
end

win = windows_rules.reject { |r| existing[r["id"]] }
lin = linux_rules.reject { |r| existing[r["id"]] }

# Also drop collisions between packs
ids = {}
win = win.select { |r| ids[r["id"]].nil? && (ids[r["id"]] = true) }
lin = lin.select { |r| ids[r["id"]].nil? && (ids[r["id"]] = true) }

write_pack(
  "windows",
  "Windows junk targets (temp, browsers, apps, package managers). Prefs/manager still apply.",
  win
)
write_pack(
  "linux",
  "Linux junk targets (XDG cache, browsers, apps, package managers). Prefs/manager still apply.",
  lin
)

puts "windows=#{win.size} linux=#{lin.size} (skipped existing collisions)"
