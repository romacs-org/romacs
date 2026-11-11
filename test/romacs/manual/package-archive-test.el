;;; package-archive-test.el --- Test HTTPS package archive access via TLS -*- lexical-binding: t -*-

;; Test that romacs can fetch package archives over HTTPS using its
;; native rustls-backed TLS implementation.  Configures package.el to
;; use the Tsinghua mirror and attempts to download archive contents
;; from gnu, nongnu, and melpa.
;;
;; Usage: ./target/release/romacs -Q -l test/romacs/manual/package-archive-test.el

;;; Code:

(require 'package)

(defvar package-archive-test--buffer-name "*Package Archive Test*"
  "Name of the test buffer.")

(defvar package-archive-test--failures 0
  "Number of failed checks so far.")

(defvar package-archive-test--passes 0
  "Number of passed checks so far.")

(defun package-archive-test--heading (title)
  "Insert a section heading TITLE."
  (insert "\n")
  (let ((start (point)))
    (insert (format "=== %s ===\n" title))
    (put-text-property start (point) 'face '(:weight bold :foreground "gold" :height 1.2))))

(defun package-archive-test--ok (description)
  "Record a PASS for DESCRIPTION."
  (setq package-archive-test--passes (1+ package-archive-test--passes))
  (let ((start (point)))
    (insert (format "  PASS: %s\n" description))
    (put-text-property start (point) 'face '(:foreground "lime green"))))

(defun package-archive-test--fail (description &optional detail)
  "Record a FAIL for DESCRIPTION with optional DETAIL."
  (setq package-archive-test--failures (1+ package-archive-test--failures))
  (let ((start (point)))
    (insert (format "  FAIL: %s\n" description))
    (put-text-property start (point) 'face '(:foreground "red" :weight bold)))
  (when detail
    (insert (format "        %s\n" detail))))

(defun package-archive-test--check-tls-available ()
  "Check that TLS is available via the GnuTLS-compatible facade."
  (package-archive-test--heading "1. TLS Availability")
  (let ((caps (gnutls-available-p)))
    (if caps
        (package-archive-test--ok
         (format "gnutls-available-p returned: %s" caps))
      (package-archive-test--fail "gnutls-available-p returned nil")))
  (if (gnutls-available-p)
      (package-archive-test--ok "TLS is available for HTTPS connections")
    (package-archive-test--fail "TLS not available -- HTTPS will not work")))

(defun package-archive-test--check-romacs-tls ()
  "Check romacs-specific TLS availability."
  (package-archive-test--heading "2. Romacs TLS Backend")
  (if (fboundp 'romacs-tls-available-p)
      (let ((result (romacs-tls-available-p)))
        (if result
            (package-archive-test--ok "romacs-tls-available-p => t")
          (package-archive-test--fail "romacs-tls-available-p => nil")))
    (package-archive-test--ok "romacs-tls-available-p not present (optional)")))

(defun package-archive-test--configure-archives ()
  "Configure package archives to use Tsinghua HTTPS mirror."
  (package-archive-test--heading "3. Configure Package Archives")
  (setq package-archives
        '(("gnu"    . "https://mirrors.tuna.tsinghua.edu.cn/elpa/gnu/")
          ("nongnu" . "https://mirrors.tuna.tsinghua.edu.cn/elpa/nongnu/")
          ("melpa"  . "https://mirrors.tuna.tsinghua.edu.cn/elpa/melpa/")))
  (setq package-check-signature nil)
  (setq package-archive-priorities
        '(("melpa" . 1) ("nongnu" . 5) ("gnu" . 10)))
  (package-archive-test--ok
   (format "Configured %d archive(s): %s"
           (length package-archives)
           (mapconcat #'car package-archives ", ")))
  (package-archive-test--ok "package-check-signature => nil")
  (package-archive-test--ok
   (format "Archive priorities: %s"
           (mapconcat (lambda (p) (format "%s=%d" (car p) (cdr p)))
                      package-archive-priorities ", "))))

(defun package-archive-test--fetch-archive (name url)
  "Attempt to fetch archive NAME from URL and report results."
  (insert (format "\n  Fetching %s from %s ...\n" name url))
  (condition-case err
      (let ((buffer (url-retrieve-synchronously url t t 30)))
        (if buffer
            (let ((status (with-current-buffer buffer
                            (buffer-string))))
              (if (string-match "200 OK" status)
                  (progn
                    (package-archive-test--ok
                     (format "%s archive fetched successfully (HTTP 200, %d bytes)"
                             name (length status)))
                    (let ((body (with-current-buffer buffer
                                  (goto-char (point-min))
                                  (re-search-forward "^$" nil t)
                                  (buffer-substring (point) (point-max)))))
                      (if (string-match-p "\"pkg\"" body)
                          (package-archive-test--ok
                           (format "%s archive body contains package data" name))
                        (package-archive-test--fail
                         (format "%s archive body missing expected package data" name)
                         (format "body preview: %s"
                                 (substring body 0 (min 200 (length body))))))))
                (package-archive-test--fail
                 (format "%s archive returned non-200 status" name)
                 (format "response: %s" (substring status 0 (min 300 (length status))))))
              (kill-buffer buffer))
          (package-archive-test--fail
           (format "%s archive fetch returned nil buffer" name))))
    (error
     (package-archive-test--fail
      (format "%s archive fetch errored: %s" name (error-message-string err))))))

(defun package-archive-test--fetch-all-archives ()
  "Fetch all configured package archives over HTTPS."
  (package-archive-test--heading "4. Fetch Archives Over HTTPS")
  (dolist (archive package-archives)
    (let ((name (car archive))
          (url (concat (cdr archive) "archive-contents")))
      (package-archive-test--fetch-archive name url))))

(defun package-archive-test--refresh-contents ()
  "Run package-refresh-contents to exercise the full package pathway."
  (package-archive-test--heading "5. package-refresh-contents")
  (condition-case err
      (progn
        (package-refresh-contents t)
        (package-archive-test--ok "package-refresh-contents completed"))
    (error
     (package-archive-test--fail
      (format "package-refresh-contents errored: %s"
              (error-message-string err))))))

(defun package-archive-test--check-packages ()
  "Verify that some well-known packages are available after refresh."
  (package-archive-test--heading "6. Verify Package Catalog")
  (let ((known-packages '("magit" "use-package" "which-key" "company" "ivy")))
    (dolist (pkg known-packages)
      (if (assoc (intern pkg) package-archive-contents)
          (package-archive-test--ok (format "Package \"%s\" found in catalog" pkg))
        (package-archive-test--fail (format "Package \"%s\" not found in catalog" pkg))))))

(defun package-archive-test-run ()
  "Run all package archive HTTPS tests."
  (let ((buf (get-buffer-create package-archive-test--buffer-name)))
    (switch-to-buffer buf)
    (let ((inhibit-read-only t))
      (erase-buffer)
      (setq package-archive-test--failures 0)
      (setq package-archive-test--passes 0)

      (let ((start (point)))
        (insert "PACKAGE ARCHIVE HTTPS / TLS TEST\n")
        (put-text-property start (point)
                           'face '(:weight bold :height 1.8 :foreground "cyan")))
      (insert (format "Window system: %s\n" window-system))
      (insert (make-string 72 ?-) "\n")
      (insert "Tests TLS connectivity by fetching ELPA archives over HTTPS.\n")

      (package-archive-test--check-tls-available)
      (package-archive-test--check-romacs-tls)
      (package-archive-test--configure-archives)
      (package-archive-test--fetch-all-archives)
      (package-archive-test--refresh-contents)
      (package-archive-test--check-packages)

      (insert "\n")
      (let ((start (point)))
        (insert (make-string 72 ?=) "\n")
        (put-text-property start (point) 'face '(:foreground "gold")))
      (let ((total (+ package-archive-test--passes package-archive-test--failures)))
        (insert (format "Results: %d/%d passed, %d failed\n"
                        package-archive-test--passes total
                        package-archive-test--failures))
        (if (zerop package-archive-test--failures)
            (message "ALL PACKAGE ARCHIVE TESTS PASSED (%d checks)" total)
          (message "PACKAGE ARCHIVE TESTS: %d FAILURES out of %d checks"
                   package-archive-test--failures total)))

      (goto-char (point-min))
      (setq buffer-read-only t))))

  (if (zerop package-archive-test--failures)
      (kill-emacs 0)
    (kill-emacs 1)))

(package-archive-test-run)

;;; package-archive-test.el ends here
