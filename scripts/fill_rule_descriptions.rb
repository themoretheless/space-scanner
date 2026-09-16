#!/usr/bin/env ruby
# frozen_string_literal: true

# Fill only missing cleanup-rule descriptions while preserving YAML formatting.

require "yaml"

ROOT = File.expand_path("..", __dir__)

KIND_TEXT = {
  "logs" => "old logs and diagnostic output",
  "temp" => "temporary files",
  "leftovers" => "leftover application data",
  "package-cache" => "downloaded package-manager cache data",
  "browser-cache" => "regenerable browser cache data",
  "app-cache" => "regenerable application cache data",
  "media-cache" => "regenerable media cache data",
  "ml-cache" => "regenerable machine-learning cache data",
  "ml-models" => "downloaded machine-learning model data",
  "vm-cache" => "regenerable virtual-machine cache data",
  "vm-disk" => "virtual-machine disk data",
  "game-cache" => "regenerable game cache data",
  "shader-cache" => "compiled shader cache data",
  "wine-cache" => "regenerable Wine compatibility cache data",
  "databases" => "application database data",
  "dev" => "regenerable developer-tool output",
  "system" => "system-managed data",
  "caches" => "regenerable cache data"
}.freeze

def description_for(rule)
  name = rule.fetch("name").to_s.sub(/\.$/, "")
  category = rule.fetch("category", "misc").to_s
  subject = KIND_TEXT.fetch(category, "data matched by this rule")
  age = rule.fetch("min_age_days", 0).to_i
  risk = rule.fetch("risk", "low").to_s

  if rule["report_only"] || risk == "report-only"
    "Inventories #{subject} for #{name}; report only, never deleted."
  else
    age_text = age.positive? ? " Files newer than #{age} days are kept." : ""
    caution = case risk
              when "high"
                " Dangerous: inspect every match before moving it to Trash."
              when "medium"
                " Review matches before moving them to Trash."
              else
                " Matched data is expected to be regenerable."
              end
    "Finds #{subject} for #{name}.#{age_text}#{caution}"
  end
end

def scalar(block, key, default = nil)
  line = block.find { |item| item.match?(/^(?:-\s+|\s{2})#{Regexp.escape(key)}:/) }
  return default unless line

  value = line.split(":", 2).last.to_s.strip
  if (value.start_with?('"') && value.end_with?('"')) ||
     (value.start_with?("'") && value.end_with?("'"))
    value = value[1...-1]
  end
  value
end

updated = 0

Dir[File.join(ROOT, "rules", "**", "*.{yaml,yml}")].sort.each do |path|
  next if File.basename(path).start_with?("_protected-")

  lines = File.readlines(path, chomp: false)
  starts = lines.each_index.select { |i| lines[i].start_with?("- ") }
  next if starts.empty?

  starts.reverse_each do |start|
    finish = starts.find { |i| i > start } || lines.length
    block = lines[start...finish]
    description_index = block.index { |line| line.match?(/^\s{2}description:/) }
    next if description_index && !scalar(block, "description", "").empty?

    name = scalar(block, "name")
    next if name.to_s.empty?
    rule = {
      "name" => name,
      "category" => scalar(block, "category", "misc"),
      "min_age_days" => scalar(block, "min_age_days", "0").to_i,
      "risk" => scalar(block, "risk", "low"),
      "report_only" => scalar(block, "report_only", "false") == "true"
    }

    description = description_for(rule)
    if description_index
      lines[start + description_index] = "  description: #{description.inspect}\n"
      updated += 1
      next
    end

    insertion = block.index { |line| line.match?(/^\s{2}name:/) }
    insertion ||= 0 if block.first.match?(/^- name:/)
    abort "cannot locate name in #{path}:#{start + 1}" unless insertion

    lines.insert(start + insertion + 1, "  description: #{description.inspect}\n")
    updated += 1
  end

  File.write(path, lines.join)
end

puts "filled descriptions: #{updated}"
