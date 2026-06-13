# Short-hand for running `miniserve` to serve `wisp-editor-web` locally.
{ writeShellScriptBin
, wisp-editor-web
, miniserve
,
}:
writeShellScriptBin "serve-wisp-editor-web" ''
  ${miniserve}/bin/miniserve \
    --index ${wisp-editor-web}/index.html \
    --disable-indexing \
    --hide-version-footer \
    --hide-theme-selector \
    --header "Cross-Origin-Opener-Policy:same-origin" \
    --header "Cross-Origin-Embedder-Policy:require-corp" \
    --header "Cache-Control:no-store, no-cache, must-revalidate" \
    --header "Pragma:no-cache" \
    --header "Expires:0" \
    -i 0.0.0.0 \
    --port 8088 \
    ${wisp-editor-web}
''
