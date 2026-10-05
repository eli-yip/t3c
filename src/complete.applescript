on run argv
    set targetID to item 1 of argv
    tell application "Things3"
        with timeout of 30 seconds
            set targetTask to to do id targetID
            if class of targetTask is project then error "The ID belongs to a project."
            if status of targetTask is canceled then error "The to-do is canceled."
            if status of targetTask is completed then return "unchanged"
            set status of targetTask to completed
            if status of targetTask is not completed then error "Things did not confirm completion."
            return "changed"
        end timeout
    end tell
end run
