# Optional package test. Install dplyr from the Packages tab first.
# This script will not install or change any packages.

if (requireNamespace("dplyr", quietly = TRUE)) {
  sales <- data.frame(
    product = c("Coffee", "Tea", "Cocoa"),
    units = c(12, 7, 18)
  )

  top_sales <- dplyr::filter(sales, units > 10)
  cat("Products with more than 10 units sold:\n")
  print(top_sales)
} else {
  cat("dplyr is not installed. Open Packages, install dplyr, and run this script again.\n")
}
