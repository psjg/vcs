;;; drive.el --- start Claude Code over ACP in Emacs, type one prompt, quit  -*- lexical-binding: t -*-
;; Paced for a recording.  Reads its stage from the environment, set by
;; demo.sh: V0ACP_ELPA (package dir holding agent-shell), V0ACP_CMD (the
;; ACP adapter, claude-agent-acp), V0ACP_REPO (the stage), V0ACP_LOG (the
;; call log of the v0 stub).
(setq package-user-dir (getenv "V0ACP_ELPA"))
(package-initialize)
(require 'agent-shell)
(setq acp-logging-enabled nil
      agent-shell-session-strategy 'new
      agent-shell-anthropic-claude-acp-command (list (getenv "V0ACP_CMD")))
(setq default-directory (file-name-as-directory (getenv "V0ACP_REPO")))
(agent-shell-anthropic-start-claude-code)
(delete-other-windows)
(defvar drive-shell (current-buffer))
(defvar drive-t0 (float-time))
(defvar drive-sent nil)
(defvar drive-done nil)
(run-with-timer 3 1
  (lambda ()
    (let ((dt (- (float-time) drive-t0)))
      (when (and (not drive-sent) (> dt 12))
        (setq drive-sent t)
        (agent-shell-insert :text "Change hello to world in a.txt." :submit t
                            :shell-buffer drive-shell))
      ;; Done when v0 has seen a call and the prompt is back, or after 150 s.
      (when (and drive-sent (not drive-done)
                 (or (> dt 150)
                     (and (> (file-attribute-size (file-attributes (getenv "V0ACP_LOG"))) 0)
                          (with-current-buffer drive-shell
                            (string-match-p "Claude> *\\'"
                              (buffer-substring-no-properties
                               (max 1 (- (point-max) 40)) (point-max)))))))
        (setq drive-done dt)
        (run-with-timer 6 nil (lambda () (kill-emacs 0)))))))
