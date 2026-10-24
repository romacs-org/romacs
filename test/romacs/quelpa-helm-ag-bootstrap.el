;;; quelpa-helm-ag-bootstrap.el --- Bootstrap quelpa and quelpa-use-package -*- lexical-binding: t -*-

(require 'package)

(setq package-archives
      '(("gnu"    . "https://mirrors.tuna.tsinghua.edu.cn/elpa/gnu/")
        ("nongnu" . "https://mirrors.tuna.tsinghua.edu.cn/elpa/nongnu/")
        ("melpa"  . "https://mirrors.tuna.tsinghua.edu.cn/elpa/melpa/")))
(setq package-check-signature nil)
(setq package-archive-priorities
      '(("melpa" . 1) ("nongnu" . 5) ("gnu" . 10)))
(package-initialize)
(package-refresh-contents)
(message "BOOT: package.el configured")

(package-install 'helm)
(message "BOOT: helm installed")

(package-install 'quelpa)
(require 'quelpa)
(setq quelpa-update-melpa-p nil)
(setq quelpa-checkout-melpa-p nil)
(setq quelpa-melpa-recipe-stores nil)
(message "BOOT: quelpa loaded")

(package-install 'quelpa-use-package)
(require 'quelpa-use-package)
(message "BOOT: quelpa-use-package loaded, :quelpa registered: %S"
         (memq :quelpa use-package-keywords))

(provide 'quelpa-helm-ag-bootstrap)
