;;; install.el --- fetch agent-shell (and its deps acp, shell-maker) from MELPA
;; Run by demo.sh when V0ACP_ELPA holds no agent-shell yet.
(setq package-user-dir (getenv "V0ACP_ELPA")
      package-archives '(("melpa" . "https://melpa.org/packages/")
                        ("gnu" . "https://elpa.gnu.org/packages/")))
(package-initialize)
(package-refresh-contents)
(package-install 'agent-shell)
