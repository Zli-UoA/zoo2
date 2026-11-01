package zoo

/** Platform collation follows the locale used by the React candidate and member lists. */
expect fun compareLocaleText(
    left: String,
    right: String,
    locale: String,
    numeric: Boolean = false,
    baseSensitivity: Boolean = false,
): Int
