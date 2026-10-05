on run argv
    set targetID to item 1 of argv
    tell application "Things3"
        with timeout of 30 seconds
            if status of to do id targetID is completed then return "completed"
            return "not-completed"
        end timeout
    end tell
end run
