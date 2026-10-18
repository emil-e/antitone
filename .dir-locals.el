;; Analyze the production application, not only the default numerical core.
((rust-mode
  . ((eglot-workspace-configuration
      . (:rust-analyzer (:cargo (:features ["editor"]))))))
 (rust-ts-mode
  . ((eglot-workspace-configuration
      . (:rust-analyzer (:cargo (:features ["editor"])))))))
