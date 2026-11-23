fun greet(
    name: String,
): String {
    return listOf(
        "Hello",
        name,
    ).joinToString(" ")
}

println(greet("Kotlin"))
