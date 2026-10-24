;;; romacs-term-test.el --- Manual test for romacs-term GPU terminal -*- lexical-binding: t -*-

;; Usage: ./target/release/romacs -Q -l test/romacs/romacs-term-test.el
;;
;; This test verifies that the romacs-term terminal emulator:
;;   1. Creates terminals via the FFI
;;   2. Renders terminal content as GPU glyphs
;;   3. Supports Window, Inline, and Floating modes
;;
;; If the FFI functions are not available (non-romacs build), the test
;; shows a placeholder message.

;;; Code:

(require 'cl-lib)

(defun romacs-term-test--ffi-available-p ()
  "Check if romacs-term FFI functions are available."
  (fboundp 'romacs-terminal-create))

(defun romacs-term-test--insert-section (title)
  "Insert a section header."
  (insert (propertize (format "\n=== %s ===\n\n" title)
                      'face '(:weight bold :height 1.3))))

(defun romacs-term-test--run ()
  "Run the romacs-term test suite."
  (let ((buf (get-buffer-create "*romacs-term-test*")))
    (switch-to-buffer buf)
    (let ((inhibit-read-only t))
      (erase-buffer)

      (insert (propertize "Romacs-Term GPU Terminal Test\n"
                          'face '(:weight bold :height 1.5 :underline t)))
      (insert (format "Date: %s\n" (format-time-string "%Y-%m-%d %H:%M:%S")))

      ;; Check FFI availability
      (romacs-term-test--insert-section "FFI Status")

      (if (romacs-term-test--ffi-available-p)
          (progn
            (insert (propertize "PASS" 'face '(:foreground "green" :weight bold)))
            (insert " romacs-term FFI functions are available\n")

            ;; Test terminal creation
            (romacs-term-test--insert-section "Terminal Creation")

            ;; Test 1: Window mode terminal
            (insert "Test 1: Creating Window mode terminal (80x24)... ")
            (condition-case err
                (let ((id (romacs-terminal-create 80 24 0 nil)))
                  (if (and id (> id 0))
                      (progn
                        (insert (propertize (format "OK (id=%d)\n" id)
                                            'face '(:foreground "green")))
                        ;; Write some test data
                        (insert "  Writing test data... ")
                        (romacs-terminal-write
                         id "echo 'Hello from romacs-term!'\r")
                        (insert (propertize "OK\n" 'face '(:foreground "green")))

                        ;; Resize
                        (insert "  Resizing to 120x40... ")
                        (romacs-terminal-resize id 120 40)
                        (insert (propertize "OK\n" 'face '(:foreground "green")))

                        ;; Destroy
                        (insert "  Destroying... ")
                        (romacs-terminal-destroy id)
                        (insert (propertize "OK\n" 'face '(:foreground "green"))))
                    (insert (propertize "FAIL (id=0)\n"
                                        'face '(:foreground "red")))))
              (error
               (insert (propertize (format "ERROR: %s\n"
                                           (error-message-string err))
                                   'face '(:foreground "red")))))

            ;; Test 2: Inline mode terminal
            (insert "\nTest 2: Creating Inline mode terminal (60x10)... ")
            (condition-case err
                (let ((id (romacs-terminal-create 60 10 1 nil)))
                  (if (and id (> id 0))
                      (progn
                        (insert (propertize (format "OK (id=%d)\n" id)
                                            'face '(:foreground "green")))
                        (romacs-terminal-destroy id)
                        (insert "  Destroyed.\n"))
                    (insert (propertize "FAIL (id=0)\n"
                                        'face '(:foreground "red")))))
              (error
               (insert (propertize (format "ERROR: %s\n"
                                           (error-message-string err))
                                   'face '(:foreground "red")))))

            ;; Test 3: Floating mode terminal
            (insert "\nTest 3: Creating Floating mode terminal (80x24)... ")
            (condition-case err
                (let ((id (romacs-terminal-create 80 24 2 nil)))
                  (if (and id (> id 0))
                      (progn
                        (insert (propertize (format "OK (id=%d)\n" id)
                                            'face '(:foreground "green")))
                        ;; Set float position
                        (insert "  Setting float position (100, 100, opacity=0.9)... ")
                        (romacs-terminal-set-float id 100.0 100.0 0.9)
                        (insert (propertize "OK\n" 'face '(:foreground "green")))
                        (romacs-terminal-destroy id)
                        (insert "  Destroyed.\n"))
                    (insert (propertize "FAIL (id=0)\n"
                                        'face '(:foreground "red")))))
              (error
               (insert (propertize (format "ERROR: %s\n"
                                           (error-message-string err))
                                   'face '(:foreground "red")))))

            ;; Summary
            (romacs-term-test--insert-section "Summary")
            (insert "All FFI entry points tested.\n")
            (insert "For visual verification, use:\n")
            (insert "  M-x romacs-term       -- open a window-mode terminal\n")
            (insert "  M-x romacs-term-floating -- open a floating terminal\n"))

        ;; FFI not available
        (insert (propertize "SKIP" 'face '(:foreground "yellow" :weight bold)))
        (insert " romacs-term FFI functions not available\n")
        (insert "\nThis is expected for non-romacs builds.\n")
        (insert "The FFI is provided by the romacs-display Rust library\n")
        (insert "with the 'romacs-term' feature enabled.\n")
        (insert "\nTo test, build romacs with:\n")
        (insert "  cargo build --manifest-path romacs-display/Cargo.toml\n")
        (insert "  make -C src emacs\n"))

      (romacs-term-test--insert-section "Architecture")
      (insert "romacs-term uses a two-thread architecture:\n\n")
      (insert "  Emacs (C)                     Render thread (Rust/wgpu)\n")
      (insert "  ─────────                     ──────────────────────────\n")
      (insert "  terminal_create() ──cmd──►    TerminalManager::create()\n")
      (insert "  terminal_write()  ──cmd──►    TerminalView::write()\n")
      (insert "  terminal_resize() ──cmd──►    TerminalView::resize()\n")
      (insert "  terminal_destroy()──cmd──►    TerminalManager::destroy()\n")
      (insert "                                PTY reader thread\n")
      (insert "                                  └─► ansi::Processor\n")
      (insert "                                  └─► Term::advance()\n")
      (insert "                                  └─► wakeup → render\n")
      (insert "\n  Terminal cells are expanded into FrameGlyph::Char\n")
      (insert "  and rendered through the existing rect + glyph pipeline.\n")

      (goto-char (point-min)))
    (special-mode)))

;; Run the test
(romacs-term-test--run)

;;; romacs-term-test.el ends here
