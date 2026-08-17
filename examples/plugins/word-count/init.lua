local function show_stats()
    local buffer = kanso.editor.current_buffer()
    local text = buffer:text()

    local words = 0

    for _ in text:gmatch("%S+") do
        words = words + 1
    end

    kanso.ui.notify(
        string.format("%s — %d words", buffer:name(), words)
    )
end

kanso.commands.register("stats.words", show_stats)

kanso.keymap.bind("alt+w", "stats.words")

kanso.events.subscribe("buffer_saved", function(event)
    local buffer = kanso.editor.current_buffer()

    kanso.ui.set_status_message(
        string.format("Saved %s (buffer %d)", buffer:name(), event.buffer_id),
        1200
    )
end)
