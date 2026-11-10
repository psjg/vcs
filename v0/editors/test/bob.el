;;; bob.el --- Bob, in Emacs, joining.  Driven by v0/tests/editors.rs.  -*- lexical-binding: t -*-
(let* ((dir (getenv "V0_TEST_DIR"))
       (file (lambda (name) (expand-file-name name dir)))
       (mark (lambda (name body) (write-region body nil (funcall file name) nil 'silent)))
       (await (lambda (what cond)
                (let ((until (+ (float-time) 15)))
                  (while (not (funcall cond))
                    (when (> (float-time) until)
                      (funcall mark "bob.failed"
                               (format "waiting for %s; buffer:\n%s\nerror: %s" what (buffer-string) v0-live-error))
                      (kill-emacs 1))
                    (accept-process-output nil 0.01)))))
       (has (lambda (s) (save-excursion (goto-char (point-min)) (search-forward s nil t)))))
  (load (expand-file-name "emacs/v0-live.el" (getenv "V0_EDITORS")) nil t)
  (setq v0-live-bin (getenv "V0_BIN"))
  (find-file (funcall file "bob/demo.rs"))
  (v0-live-start (funcall file "s"))
  (funcall await "ready" (lambda () (equal v0-live-ready "guest")))
  (funcall await "the document" (lambda () (funcall has "fn foo() {}")))

  ;; Live, both ways.
  (goto-char (point-max))
  (insert "// bob here\n")
  (funcall await "alice's line" (lambda () (funcall has "// alice hère")))

  ;; Offline: one more call to foo.
  (v0-live-hold)
  (funcall mark "bob.held" "")
  (funcall await "alice to rename" (lambda () (file-exists-p (funcall file "alice.renamed"))))
  (goto-char (point-min))
  (search-forward "    foo();\n")
  (insert "    foo();\n")
  (funcall mark "bob.typed" (buffer-string))
  (v0-live-release)

  (funcall await "his call renamed" (lambda () (funcall has "    bar();\n    bar();")))
  (funcall mark "bob.out" (buffer-string))
  (kill-emacs 0))
