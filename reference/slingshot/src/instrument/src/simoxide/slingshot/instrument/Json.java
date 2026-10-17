package simoxide.slingshot.instrument;

/** Minimal JSON line builder in the palladio-trace/1 conventions (docs/guide/formats.md §1). */
public final class Json {
	private final StringBuilder sb = new StringBuilder(160);
	private boolean first = true;

	public Json() {
		sb.append('{');
	}

	private void key(final String k) {
		if (!first) {
			sb.append(',');
		}
		first = false;
		sb.append('"').append(k).append("\":");
	}

	public Json str(final String k, final String v) {
		if (v == null) {
			return this;
		}
		key(k);
		quote(sb, v);
		return this;
	}

	public Json num(final String k, final double v) {
		key(k);
		num(sb, v);
		return this;
	}

	public Json integer(final String k, final long v) {
		key(k);
		sb.append(v);
		return this;
	}

	public Json bool(final String k, final boolean v) {
		key(k);
		sb.append(v);
		return this;
	}

	/** Appends an already serialised JSON value. */
	public Json raw(final String k, final String json) {
		key(k);
		sb.append(json);
		return this;
	}

	public String end() {
		return sb.append('}').toString();
	}

	/** Java {@code Double.toString} (JDK >= 19: shortest round trip); NaN and infinities as strings. */
	public static void num(final StringBuilder sb, final double v) {
		if (Double.isNaN(v) || Double.isInfinite(v)) {
			sb.append('"').append(Double.toString(v)).append('"');
		} else {
			sb.append(Double.toString(v));
		}
	}

	public static String num(final double v) {
		final StringBuilder sb = new StringBuilder();
		num(sb, v);
		return sb.toString();
	}

	public static void quote(final StringBuilder sb, final String s) {
		sb.append('"');
		for (int i = 0; i < s.length(); i++) {
			final char c = s.charAt(i);
			switch (c) {
			case '"' -> sb.append("\\\"");
			case '\\' -> sb.append("\\\\");
			case '\n' -> sb.append("\\n");
			case '\r' -> sb.append("\\r");
			case '\t' -> sb.append("\\t");
			default -> {
				if (c < 0x20) {
					sb.append(String.format("\\u%04x", (int) c));
				} else {
					sb.append(c);
				}
			}
			}
		}
		sb.append('"');
	}

	public static String quote(final String s) {
		final StringBuilder sb = new StringBuilder();
		quote(sb, s);
		return sb.toString();
	}
}
