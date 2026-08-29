import re

with open("kizuna-voice/src/audio/scheduler.rs", "r") as f:
    code = f.read()

# Change initial state to Playing
code = code.replace("let mut state = TrackState::Idle;", "let mut state = TrackState::Playing;\n        let _ = event_tx.send(TrackEvent::Started);")

# Remove MissedTickBehavior::Skip
code = re.sub(r'interval\.set_missed_tick_behavior\(MissedTickBehavior::Skip\);\n', '', code)

with open("kizuna-voice/src/audio/scheduler.rs", "w") as f:
    f.write(code)
