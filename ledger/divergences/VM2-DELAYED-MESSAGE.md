## Observable by a package

| # | mechanism | GNU | romacs | why |
|---|---|---|---|---|
| VM2-DELAYED-MESSAGE | `funcall-with-delayed-message` (`with-delayed-message`) | an atimer shows MESSAGE at the evaluator's first safepoint past TIMEOUT, even while FUNCTION computes | a timer shows it, so only once FUNCTION waits for input or a process past TIMEOUT; FUNCTION's value and errors are GNU's | the main VM runs no timers and no redisplay while Lisp computes; a message it could show there would not be painted | <!-- symbols: funcall-with-delayed-message, with-delayed-message -->

## VM2-DELAYED-MESSAGE — The delayed message waits for a wait

GNU's eval.c `Ffuncall_with_delayed_message` starts an atimer, which shows
MESSAGE with `message3` when it fires, at the next `maybe_quit`. Romacs's main
VM runs timers and redisplays only where it waits, so the function is Lisp
(lisp/romacs/vm/prelude.el) over `run-with-timer`, cancelled when FUNCTION
returns or exits non-locally. consult's `consult--slow-operation` and
markdown-mode reach it; for them the value is what matters, and a long
computation that never waits shows no message.
