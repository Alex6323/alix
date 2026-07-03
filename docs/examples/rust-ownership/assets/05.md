In addition, there’s a design choice that’s implied by this: Rust will never
automatically create “deep” copies of your data. Therefore, any _automatic_
copying can be assumed to be inexpensive in terms of runtime performance.
