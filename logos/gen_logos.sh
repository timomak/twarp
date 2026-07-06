#!/bin/zsh
set -e
API_KEY="$GEMINI_API_KEY"
MODEL="gemini-3-pro-image-preview"
OUT_DIR="/Users/thirdfacedev/Development/twarp/logos"

gen() {
  local name="$1"
  local prompt="$2"
  echo "Generating $name..."
  jq -n --arg p "$prompt" '{contents:[{parts:[{text:$p}]}],generationConfig:{responseModalities:["IMAGE"]}}' > /tmp/req.json
  curl -s -X POST \
    "https://generativelanguage.googleapis.com/v1beta/models/${MODEL}:generateContent" \
    -H "x-goog-api-key: ${API_KEY}" \
    -H "Content-Type: application/json" \
    -d @/tmp/req.json > /tmp/resp_${name}.json
  jq -r '.candidates[0].content.parts[] | select(.inlineData) | .inlineData.data' /tmp/resp_${name}.json | base64 -d > /tmp/${name}.raw
  sips -s format png /tmp/${name}.raw --out "${OUT_DIR}/${name}.png" > /dev/null
  ls -la "${OUT_DIR}/${name}.png"
}

gen "twarp-logo-1-pixel-arcade" "Design a square app logo for a terminal/IDE app called 'twarp'. Style: 1990s pixel-art arcade aesthetic. Chunky pixel lettering spelling 'twarp' in lowercase, neon magenta and cyan on a deep purple-black background, with a blocky pixelated warp-tunnel or portal motif behind the text, subtle CRT scanlines. Clean, iconic, centered composition, flat background, no photorealism."

gen "twarp-logo-2-vaporwave-chrome" "Design a square app logo for a terminal/IDE app called 'twarp'. Style: neo-retro 90s vaporwave. The word 'twarp' in shiny beveled chrome 3D lettering like 90s video game box art, floating over a sunset gradient grid horizon (synthwave laser grid), with a glowing wireframe warp portal ring behind it. Colors: teal, hot pink, orange sunset, dark indigo sky. Iconic, centered, poster-quality, no photorealism."

gen "twarp-logo-3-terminal-crt" "Design a square app logo for a terminal/IDE app called 'twarp'. Style: retro 1990s computer terminal. A chunky old beige CRT monitor icon drawn in bold flat retro-tech illustration style, screen showing a glowing green phosphor command prompt with the text 'twarp_' and a blinking cursor, warp-speed starfield streaks radiating from the screen. Memphis-design accents, thick outlines, limited retro palette (beige, green, black, one accent red). Clean sticker-like logo on a solid dark background."
