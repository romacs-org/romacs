/* Outer /* nested */ still outer */
interface Greeting {
    fun greet(name: String): String
}

@Deprecated("Example")
class Greeter : Greeting {
    val prefix = "Hello"
    var count = 0

    override fun greet(name: String): String {
        // Count each greeting.
        count += 1
        return "${prefix}, $name"
    }
}

object Examples {
    val quote = '"'
    val raw = """
        "quoted" // literal text /* literal */
        a backslash \
        """
    val empty = """"""
    val trailingQuote = """Ends in a quote""""
    val done = true
}
