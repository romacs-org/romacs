fun greet(
    name: String,
): String {
    return listOf(
        "Hello",
        name,
    ).joinToString(" ")
}

println(greet("Kotlin"))

fun classify(n: Int) {
    when (n) {
        1 ->
            println(1)
        else ->
            println(0)
    }
    if (n == 1)
        println(1)
    else if (n == 2)
        println(2)
    else
        println(0)
}
