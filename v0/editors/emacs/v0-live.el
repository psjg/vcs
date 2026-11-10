;;; v0-live.el --- this buffer as one replica of a shared v0 document  -*- lexical-binding: t -*-

;;; Commentary:

;; The session (`v0 live', v0/src/live.rs) owns the document; this file owns
;; nothing but the buffer.  It reports what was typed as "replace DEL
;; characters at AT with TEXT" and applies the same from peers.
;;
;;   M-x v0-live-start RET /tmp/x.sock   host if no one is there, else join
;;   M-x v0-live-rename RET bar          rename the name at point, as an intent
;;   M-x v0-live-hold / v0-live-release  go offline / come back
;;
;; Emacs says exactly what changed (`after-change-functions'), in characters,
;; so nothing is diffed here.

;;; Code:

(defvar v0-live-bin "v0" "The v0 executable.")

(defvar-local v0-live--process nil)
(defvar-local v0-live--sent 0 "Edits and renames sent to the session.")
(defvar-local v0-live--seen 0 "The last message from the session applied.")
(defvar-local v0-live--applying nil "Non-nil while a peer's edit is going in.")
(defvar-local v0-live--pending "" "The start of a line not yet whole.")
(defvar-local v0-live-ready nil "The role, once the session is up.")
(defvar-local v0-live-error nil "The last refusal from the session.")

(defun v0-live--say (msg)
  (process-send-string v0-live--process (concat (json-serialize msg) "\n")))

(defun v0-live--typed (beg end len)
  "Tell the session that LEN characters at BEG became what is up to END."
  (unless v0-live--applying
    (setq v0-live--sent (1+ v0-live--sent))
    (v0-live--say `((edit . ((at . ,(1- beg)) (del . ,len)
                             (text . ,(buffer-substring-no-properties beg end))
                             (seen . ,v0-live--seen)))))))

(defun v0-live--heard (msg)
  (let-alist msg
    (cond
     (.ready
      (let-alist .ready
        (if (equal .role "guest")
            (let ((v0-live--applying t))
              (erase-buffer)
              (insert .text)
              (goto-char (point-min)))
          (unless (= (point-min) (point-max))
            (v0-live--typed (point-min) (point-max) 0)))
        (add-hook 'after-change-functions #'v0-live--typed nil t)
        (setq v0-live-ready .role)))
     ;; Typed since the session wrote this: it no longer fits, and the
     ;; session sends what it meant again once it has heard the edit.
     ((and .apply (= (alist-get 'acked .apply) v0-live--sent))
      (let ((v0-live--applying t))
        (save-excursion
          (seq-doseq (e (alist-get 'edits .apply))
            (let-alist e
              (goto-char (1+ .at))
              (delete-char .del)
              (insert .text)))))
      (setq v0-live--seen (alist-get 'n .apply))
      (v0-live--say `((seen . ,v0-live--seen))))
     (.error
      (setq v0-live-error .error)
      (message "v0 live: %s" .error)))))

(defun v0-live--filter (process chunk)
  (when (buffer-live-p (process-buffer process))
    (with-current-buffer (process-buffer process)
      (save-restriction
        (widen)
        ;; Chunks, not lines: the last piece is the start of the next line.
        (let ((lines (split-string (concat v0-live--pending chunk) "\n")))
          (setq v0-live--pending (car (last lines)))
          (dolist (line (butlast lines))
            (v0-live--heard (json-parse-string line :object-type 'alist))))))))

(defun v0-live-start (socket)
  "Make this buffer a replica of the document on SOCKET."
  (interactive "FSocket: ")
  (when v0-live--process (user-error "v0 live is already on in this buffer"))
  (setq v0-live--sent 0 v0-live--seen 0 v0-live--pending "")
  (setq v0-live--process
        (make-process
         :name "v0-live" :buffer (current-buffer) :noquery t
         :command (list v0-live-bin "live" (expand-file-name socket)
                        (file-name-nondirectory (or buffer-file-name (buffer-name))))
         :connection-type 'pipe :coding 'utf-8-unix
         :filter #'v0-live--filter
         :stderr (get-buffer-create "*v0-live-errors*")
         :sentinel (lambda (process _)
                     (unless (process-live-p process)
                       (when (buffer-live-p (process-buffer process))
                         (with-current-buffer (process-buffer process)
                           (remove-hook 'after-change-functions #'v0-live--typed t)
                           (setq v0-live--process nil v0-live-ready nil))))))))

(defun v0-live-rename (to)
  "Rename the name at point to TO, wherever it is meant."
  (interactive "sRename to: ")
  (setq v0-live--sent (1+ v0-live--sent))
  (v0-live--say `((rename . ((at . ,(1- (point))) (to . ,to) (seen . ,v0-live--seen))))))

(defun v0-live-hold ()
  "Go offline: keep typing, hear and tell no one."
  (interactive)
  (v0-live--say '((hold . t))))

(defun v0-live-release ()
  "Come back: send what was typed offline, take what peers did."
  (interactive)
  (v0-live--say '((hold . :false))))

(provide 'v0-live)
;;; v0-live.el ends here
