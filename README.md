# 🦀 rsStudio

rsStudio is a Linux-first, Rust-native IDE for R. It uses egui for the interface and an installed R interpreter for computation. The project is an early prototype.

## Features

- Edit and run R scripts in a persistent R session.
- Install and load common CRAN and Bioconductor packages.
- Generate R code with an optional OpenRouter assistant; review and run suggestions yourself.
- Choose a light or dark theme.

## Getting started

Install Rust, Cargo, R, and the Linux libraries required by eframe, then run:

```sh
R --version
cargo run
```

Select **Packages** to manage R packages. To use the assistant, open the **R AI** panel and enter an OpenRouter API key. R code runs with your normal account permissions.

See the [user guide](docs/user-guide.md) for setup details and the [development plan](docs/README.md) for planned features. Environment inspection, data viewing, and plots are still in development.

## License

[MIT](LICENSE). R and third-party dependencies have their own licenses. rsStudio is a working name pending trademark clearance and is not affiliated with Posit or RStudio.
