## Detail

| crate | file | entries |
|---|---|---|
| VM2-PACKAGES | package.el and package-vc.el | GNU byte compiles an installed package (`package--compile`), runs commands in the calling thread, and saves `package-selected-packages` through Custom | Romacs runs GNU's own code. `package--compile` does nothing, as packages load from source; in a running session the commands that download run in a worker VM and return at once, the session activating the result when they end; the selected list is saved beside them, since nothing writes to the init (`lisp/romacs/romacs-package.el`) | **Package-observable** for code that reads `.elc` files of a package or expects `package-install` in a running session to have finished when it returns <!-- symbols: package-install, package-vc-install, package-user-dir, package-selected-packages, package--compile --> |
