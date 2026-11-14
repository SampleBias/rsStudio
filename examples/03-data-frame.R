# Create and filter a small data frame using base R.

weather <- data.frame(
  city = c("Panama City", "David", "Boquete"),
  temperature = c(30, 32, 22)
)

cat("All observations:\n")
print(weather)

cat("\nTemperatures above 25:\n")
print(weather[weather$temperature > 25, ])

cat("\nAverage temperature:", mean(weather$temperature), "\n")
