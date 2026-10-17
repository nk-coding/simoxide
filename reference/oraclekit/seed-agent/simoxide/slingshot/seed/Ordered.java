package simoxide.slingshot.seed;

import java.util.Collection;
import java.util.Collections;
import java.util.LinkedHashSet;
import java.util.Objects;
import java.util.Set;
import java.util.stream.Collector;
import java.util.stream.Collectors;

/**
 * Insertion-ordered replacements for the JDK's hash-ordered set factories, called from Slingshot classes
 * rewritten by {@link OrderingTransformer}. Same semantics as the originals (immutability, null and
 * duplicate checks); only the iteration order differs: insertion (argument) order instead of
 * identity-hash order ({@code Collectors.toSet()}) or per-JVM salted order ({@code Set.of}).
 * Lives on the boot class path (Boot-Class-Path) and is boot-delegated by Equinox.
 */
public final class Ordered {
	private Ordered() {
	}

	public static <T> Collector<T, ?, Set<T>> toSet() {
		return Collectors.toCollection(LinkedHashSet::new);
	}

	public static <T> Collector<T, ?, Set<T>> toUnmodifiableSet() {
		return Collectors.collectingAndThen(Collectors.toCollection(LinkedHashSet::new),
				Collections::unmodifiableSet);
	}

	/** {@code Set.of(E...)}: rejects null and duplicate elements like the original. */
	@SuppressWarnings("unchecked")
	public static <E> Set<E> setOfArray(final Object[] es) {
		final LinkedHashSet<Object> s = new LinkedHashSet<>(Math.max(4, es.length * 2));
		for (final Object e : es) {
			if (!s.add(Objects.requireNonNull(e))) {
				throw new IllegalArgumentException("duplicate element: " + e);
			}
		}
		return (Set<E>) Collections.unmodifiableSet(s);
	}

	public static <E> Set<E> setCopyOf(final Collection<? extends E> c) {
		final LinkedHashSet<E> s = new LinkedHashSet<>();
		for (final E e : c) {
			s.add(Objects.requireNonNull(e));
		}
		return Collections.unmodifiableSet(s);
	}

	public static <E> Set<E> setOf() {
		return setOfArray(new Object[] {  });
	}

	public static <E> Set<E> setOf(final E a) {
		return setOfArray(new Object[] { a });
	}

	public static <E> Set<E> setOf(final E a, final E b) {
		return setOfArray(new Object[] { a, b });
	}

	public static <E> Set<E> setOf(final E a, final E b, final E c) {
		return setOfArray(new Object[] { a, b, c });
	}

	public static <E> Set<E> setOf(final E a, final E b, final E c, final E d) {
		return setOfArray(new Object[] { a, b, c, d });
	}

	public static <E> Set<E> setOf(final E a, final E b, final E c, final E d, final E e) {
		return setOfArray(new Object[] { a, b, c, d, e });
	}

	public static <E> Set<E> setOf(final E a, final E b, final E c, final E d, final E e, final E f) {
		return setOfArray(new Object[] { a, b, c, d, e, f });
	}

	public static <E> Set<E> setOf(final E a, final E b, final E c, final E d, final E e, final E f, final E g) {
		return setOfArray(new Object[] { a, b, c, d, e, f, g });
	}

	public static <E> Set<E> setOf(final E a, final E b, final E c, final E d, final E e, final E f, final E g, final E h) {
		return setOfArray(new Object[] { a, b, c, d, e, f, g, h });
	}

	public static <E> Set<E> setOf(final E a, final E b, final E c, final E d, final E e, final E f, final E g, final E h, final E i) {
		return setOfArray(new Object[] { a, b, c, d, e, f, g, h, i });
	}

	public static <E> Set<E> setOf(final E a, final E b, final E c, final E d, final E e, final E f, final E g, final E h, final E i, final E j) {
		return setOfArray(new Object[] { a, b, c, d, e, f, g, h, i, j });
	}
}
