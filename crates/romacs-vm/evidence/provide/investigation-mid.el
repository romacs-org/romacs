(with-temp-buffer (insert "x\n\n") (ensure-empty-lines))
(match-data t)
(save-match-data 1)
