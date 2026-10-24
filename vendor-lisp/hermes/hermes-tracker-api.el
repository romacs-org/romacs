;;; hermes-tracker-api.el --- Tracker HTTP boundary -*- lexical-binding: t; -*-

;; Copyright (C) 2026 Thanos Apollo

;; Author: Thanos Apollo <public@thanosapollo.org>
;; Keywords: tools, convenience
;; Package-Requires: ((emacs "29.1") (hermes "0.2.0"))

;;; Commentary:

;; Authenticated asynchronous HTTP boundary for Hermes Tracker.

;;; Code:

(require 'auth-source)
(require 'cl-lib)
(require 'json)
(require 'subr-x)
(require 'url-parse)
(require 'url-util)
(require 'hermes-promise)
(require 'hermes-dashboard-transport)
(require 'hermes-transport)

(defgroup hermes-tracker nil
  "Integration with the Hermes Tracker service."
  :group 'hermes)

(defcustom hermes-tracker-url nil
  "Base URL of the Hermes Tracker service, or nil to disable it."
  :type '(choice (const :tag "Disabled" nil) string)
  :group 'hermes-tracker)

(defcustom hermes-tracker-auth-source-user "hermes-tracker-token"
  "Auth-source user used to find the Tracker bearer credential."
  :type 'string
  :group 'hermes-tracker)

(defcustom hermes-tracker-auth-source-port "hermes-tracker"
  "Auth-source port used to find the Tracker bearer credential."
  :type 'string
  :group 'hermes-tracker)

(defcustom hermes-tracker-actor user-login-name
  "Actor sent with Tracker mutations."
  :type 'string
  :group 'hermes-tracker)

(defun hermes-tracker--normalize-base-url (value)
  "Return validated Tracker base URL from VALUE, or nil when empty."
  (and-let* ((text (and (stringp value) (string-trim value)))
             ((not (string-empty-p text))))
    (let ((parsed (url-generic-parse-url text)))
      (unless (member (url-type parsed) '("http" "https"))
        (user-error "Hermes Tracker URL must start with http:// or https://"))
      (unless (url-host parsed)
        (user-error "Hermes Tracker URL must include a host"))
      (unless (or (equal (url-type parsed) "https")
                  (hermes-dashboard-transport--loopback-host-p
                   (url-host parsed)))
        (user-error "Hermes Tracker URL must use HTTPS outside loopback"))
      (when (or (url-user parsed) (url-password parsed))
        (user-error "Hermes Tracker URL must not include credentials"))
      (when (or (cdr (url-path-and-query parsed)) (url-target parsed))
        (user-error "Hermes Tracker URL must not include query or fragment")))
    (replace-regexp-in-string "/+\\'" "" text)))

(defun hermes-tracker--base-url ()
  "Return configured Tracker base URL or signal an actionable error."
  (or (hermes-tracker--normalize-base-url hermes-tracker-url)
      (user-error "Set `hermes-tracker-url' to enable Hermes Tracker")))

(defun hermes-tracker--auth-hosts (base-url)
  "Return auth-source host candidates for BASE-URL."
  (let* ((parsed (url-generic-parse-url base-url))
         (host (url-host parsed))
         (port (url-port parsed)))
    (cl-remove-duplicates
     (delq nil (list base-url (and host port (format "%s:%d" host port)) host))
     :test #'equal)))

(defun hermes-tracker--token (base-url)
  "Return Tracker bearer token for BASE-URL from auth-source."
  (or (catch 'token
        (dolist (host (hermes-tracker--auth-hosts base-url))
          (when-let* ((entry (car (auth-source-search
                                   :host host
                                   :user hermes-tracker-auth-source-user
                                   :port hermes-tracker-auth-source-port
                                   :require '(:secret) :max 1)))
                      (secret (plist-get entry :secret))
                      (value (if (functionp secret) (funcall secret) secret))
                      (token (hermes-transport--non-empty-string value)))
            (throw 'token token))))
      (user-error
       "No Hermes Tracker token found in auth-source for %s/%s"
       hermes-tracker-auth-source-user hermes-tracker-auth-source-port)))

(defun hermes-tracker--idempotency-key (method path actor body invocation)
  "Return stable mutation key for METHOD PATH ACTOR BODY and INVOCATION."
  (concat "emacs-hermes-"
          (secure-hash 'sha256
                       (mapconcat #'identity
                                  (list invocation method path actor
                                        (json-serialize (or body '())))
                                  "\n"))))

(defun hermes-tracker--effective-actor ()
  "Return the actor identity used for Tracker mutations and cache isolation."
  (let ((actor (and (stringp hermes-tracker-actor)
                    (string-trim hermes-tracker-actor))))
    (if (and actor (not (string-empty-p actor))) actor user-login-name)))

(defun hermes-tracker--request (method path &optional body invocation)
  "Return authenticated request plist for METHOD PATH BODY and INVOCATION."
  (let* ((base-url (hermes-tracker--base-url))
         (token (hermes-tracker--token base-url))
         (actor (hermes-tracker--effective-actor))
         (mutation-p (not (equal method "GET")))
         (headers (append
                   `(("Authorization" . ,(concat "Bearer " token)))
                   (and body '(("Content-Type" . "application/json")))
                   (and mutation-p `(("X-Actor" . ,actor)))
                   (and invocation
                        `(("Idempotency-Key"
                           . ,(hermes-tracker--idempotency-key
                               method path actor body invocation)))))))
    (list :url (concat base-url path) :method method :headers headers
          :body body :secrets (list token))))

(defun hermes-tracker--request-async (method path &optional body invocation)
  "Return promise of Tracker response body for METHOD PATH BODY and INVOCATION."
  (hermes--promise-map
   (hermes-dashboard-transport--http-json-request-async
    (hermes-tracker--request method path body invocation))
   (lambda (response) (plist-get response :body))))

(defun hermes-tracker--api-path (&rest segments)
  "Return Tracker API path from percent-encoded SEGMENTS."
  (concat "/api/v1/"
          (mapconcat (lambda (segment) (url-hexify-string (format "%s" segment)))
                     segments "/")))

(defun hermes-tracker--items (value)
  "Return VALUE as a list."
  (cond
   ((null value) nil)
   ((vectorp value) (append value nil))
   ((listp value) value)
   (t (error "Expected Tracker list, got %S" value))))

(provide 'hermes-tracker-api)
;;; hermes-tracker-api.el ends here
