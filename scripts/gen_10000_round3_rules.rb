#!/usr/bin/env ruby
# frozen_string_literal: true

# Third import: AUR + Debian package names + extra Scoop buckets.
# Writes rules/generated/src10k-*.yaml without rewriting earlier imports.

require "yaml"
require "set"
require "fileutils"

ROOT = File.expand_path("..", __dir__)
RULES = File.join(ROOT, "rules")
OUT = File.join(RULES, "generated")
CHUNK = 800
TARGET = 10_000

def slug(s)
  s.to_s.downcase.gsub(/[^a-z0-9]+/, "-").gsub(/-+/, "-").sub(/^-|-$/, "")[0, 72]
end

def safe_pkg?(name)
  return false if name.nil? || name.empty?
  return false if name.length < 2 || name.length > 80
  return false if name.include?("/") || name.include?("\\") || name.include?(" ")
  return false unless name.match?(/\A[a-zA-Z0-9][a-zA-Z0-9+._-]*\z/)
  true
end

seen = Set.new
Dir.glob(File.join(RULES, "**", "*.{yaml,yml}")).each do |f|
  next if File.basename(f).start_with?("_")
  next if File.basename(f).start_with?("src10k-")
  File.read(f).scan(/^- id: (.+)$/) { |m| seen << m[0].strip }
end

rules = []
add = lambda do |id:, name:, description:, category:, platform:, paths:|
  return false if seen.include?(id)
  paths = Array(paths).map(&:to_s).reject(&:empty?).uniq
  return false if paths.empty?
  seen << id
  rules << {
    "id" => id,
    "name" => name.to_s[0, 120],
    "description" => description.to_s[0, 400],
    "category" => category,
    "platform" => platform,
    "risk" => "low",
    "default_enabled" => false,
    "min_age_days" => 14,
    "paths" => paths,
    "delete_mode" => "contents",
    "max_depth" => 6
  }
  true
end

# AUR package names → XDG cache (https://aur.archlinux.org/packages.gz)
aur = "/tmp/aur-packages.gz"
aur_added = 0
if File.exist?(aur)
  IO.popen(["gzip", "-dc", aur]) do |io|
    io.each_line do |line|
      break if aur_added >= 5500
      pkg = line.strip
      next unless safe_pkg?(pkg)
      id = "aur-cache-#{slug(pkg)}"
      next if id.empty?
      if add.call(
        id: id,
        name: "AUR cache #{pkg}",
        description: "XDG cache for Arch User Repository package #{pkg}. Not PKGBUILDs or src clones.",
        category: "caches",
        platform: "linux",
        paths: ["$HOME/.cache/#{pkg}", "$XDG_CACHE_HOME/#{pkg}"]
      )
        aur_added += 1
      end
    end
  end
end

# Debian stable package names
deb_names = "/tmp/debian-names.txt"
if File.exist?(deb_names)
  File.foreach(deb_names) do |line|
    break if rules.size >= TARGET
    pkg = line.strip
    next unless safe_pkg?(pkg)
    add.call(
      id: "deb-cache-#{slug(pkg)}",
      name: "Debian cache #{pkg}",
      description: "XDG cache directory named after Debian package #{pkg} (deb.debian.org stable main).",
      category: "caches",
      platform: "linux",
      paths: ["$HOME/.cache/#{pkg}", "$XDG_CACHE_HOME/#{pkg}"]
    )
  end
end

# Extra Scoop buckets
Dir.glob("/tmp/scoop-ScoopInstaller_*.txt").each do |list|
  File.foreach(list) do |line|
    break if rules.size >= TARGET + 500
    name = File.basename(line.strip, ".json")
    next unless safe_pkg?(name)
    add.call(
      id: "scoop2-#{slug(name)}",
      name: "Scoop #{name} logs/temp",
      description: "Additional Scoop bucket package #{name}: logs and temp (Main/Extras/Versions/Java).",
      category: "logs",
      platform: "windows",
      paths: [
        "%LOCALAPPDATA%/#{name}/Logs",
        "%LOCALAPPDATA%/#{name}/Temp",
        "%APPDATA%/#{name}/logs",
        "%TEMP%/#{name}"
      ]
    )
  end
end

if rules.size < TARGET
  warn "only #{rules.size} (need #{TARGET})"
end

# Trim to first TARGET if overshoot from scoop
# Keep extras up to TARGET+200 for collisions during load
keep = rules.first([rules.size, TARGET + 200].min)

FileUtils.mkdir_p(OUT)
Dir.glob(File.join(OUT, "src10k-*.yaml")).each { |f| File.delete(f) }
keep.each_slice(CHUNK).with_index(1) do |chunk, i|
  File.write(File.join(OUT, format("src10k-%02d.yaml", i)), YAML.dump(
    "pack" => "src10k-#{i}",
    "description" => "Third catalog import: AUR + Debian + extra Scoop. Chunk #{i}.",
    "rules" => chunk
  ))
end

puts "wrote #{keep.size} rules linux=#{keep.count { |r| r['platform']=='linux' }} windows=#{keep.count { |r| r['platform']=='windows' }}"
