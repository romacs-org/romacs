;;; mail-headers-test.el --- GNU-compatible reply headers -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later

(romacs-deftest mail-reply-keeps-notmuch-tab-separated-references ()
  (let* ((references "<root@example.invalid>\t<parent@example.invalid>")
         (wire (romacs-mail-freeze
                "From: user@example.invalid\nTo: peer@example.invalid\nSubject: Re: Fixture\n--text follows this line--\nBody\n"
                (list :in-reply-to "<parent@example.invalid>" :references references)
                "Sat, 24 Oct 2026 18:00:00 +0000" "fixture@example.invalid")))
    (romacs-should (string-match (concat "References: " references "\n") wire))
    (romacs-should (string-match "In-Reply-To: <parent@example.invalid>\n" wire))))

(romacs-deftest mail-reply-still-rejects-injected-threading-fields ()
  (romacs-should-error
   (romacs-mail-freeze
    "From: user@example.invalid\nTo: peer@example.invalid\nSubject: Fixture\n--text follows this line--\nBody\n"
    '(:in-reply-to "<parent@example.invalid>\nBcc: wrong@example.invalid")
    "Sat, 24 Oct 2026 18:00:00 +0000" "fixture@example.invalid") :type 'user-error))

(provide 'mail-headers-test)
