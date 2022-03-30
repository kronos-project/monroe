# monroe

monroe is an asynchronous [Actor Model](https://en.wikipedia.org/wiki/Actor_model)
implementation for the Rust programming language.

It is built on top of [tokio](https://tokio.rs) to meet the demands placed on actors
in the Kronos project.

## Design principles

- **No boilerplate:** Defining new actors and integrating them into your application
  should be straightforward and simple.

- **Async/await by design:** A strong emphasis is put on forwarding `async/await`
  based API design throughout the entire library.

- **Immutability embraced:** Leveraging Rust's static guarantees, actors built with
  monroe manage only their own state and never share it with the outer world.

- **Strongly typed messages:** All message types passed between actors are strongly
  typed and have unambiguous handling semantics. No `Any` values.

## License

Licensed under either of [Apache License, Version 2.0](./LICENSE-APACHE) or
[MIT License](./LICENSE-MIT) at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted for
inclusion in this crate by you, as defined in the Apache-2.0 license, shall be dual
licensed as above, without any additional terms or conditions. 
