package simoxide.slingshot.seed;

import java.lang.instrument.ClassFileTransformer;
import java.security.ProtectionDomain;
import java.util.concurrent.atomic.AtomicInteger;

import org.objectweb.asm.ClassReader;
import org.objectweb.asm.ClassVisitor;
import org.objectweb.asm.ClassWriter;
import org.objectweb.asm.Handle;
import org.objectweb.asm.MethodVisitor;
import org.objectweb.asm.Opcodes;

/**
 * Rewrites Slingshot's classes (package prefix {@code org/palladiosimulator/analyzer/slingshot/}) at load
 * time so that hash-ordered collections iterate in insertion order (the "ordered" rule of the det mode):
 * <ul>
 * <li>{@code new HashSet(..)} / {@code HashSet::new} → {@code LinkedHashSet}; {@code new HashMap(..)} /
 * {@code HashMap::new} → {@code LinkedHashMap} (same constructors; super-constructor calls of subclasses
 * are left alone);</li>
 * <li>{@code Collectors.toSet()}, {@code Collectors.toUnmodifiableSet()} → {@link Ordered};</li>
 * <li>{@code Set.of(..)} (all arities), {@code Set.copyOf(..)} → {@link Ordered}.</li>
 * </ul>
 * With the agent argument {@code qvto} the same rule also applies to QVTo and OCL ({@link #QVTO_PREFIXES}),
 * so that OCL {@code Set}s of model objects (identity hash order) iterate in insertion order.
 * Lookups, equality and contents are unchanged; only iteration order changes. Slingshot's event
 * scheduling iterates such sets (handler results), so this fixes the order of same-time events.
 */
final class OrderingTransformer implements ClassFileTransformer {

	static final String PREFIX = "org/palladiosimulator/analyzer/slingshot/";
	/** With the agent argument {@code qvto}: QVTo's and OCL's own collections (QVTo/OCL Set, mapping traces). */
	static final String[] QVTO_PREFIXES = { "org/eclipse/ocl/", "org/eclipse/m2m/qvt/oml/",
			"org/eclipse/m2m/internal/qvt/oml/" };
	static final String ORDERED = "simoxide/slingshot/seed/Ordered";
	final boolean qvto;
	final AtomicInteger classes = new AtomicInteger();
	final AtomicInteger sites = new AtomicInteger();

	OrderingTransformer(final boolean qvto) {
		this.qvto = qvto;
	}

	boolean applies(final String name) {
		if (name.startsWith(PREFIX)) {
			return true;
		}
		if (qvto) {
			for (final String p : QVTO_PREFIXES) {
				if (name.startsWith(p)) {
					return true;
				}
			}
		}
		return false;
	}

	@Override
	public byte[] transform(final Module module, final ClassLoader loader, final String name,
			final Class<?> redefined, final ProtectionDomain pd, final byte[] bytes) {
		if (name == null || !applies(name)) {
			return null;
		}
		try {
			final ClassReader cr = new ClassReader(bytes);
			final ClassWriter cw = new ClassWriter(cr, 0);
			final int before = sites.get();
			cr.accept(new ClassVisitor(Opcodes.ASM9, cw) {
				@Override
				public MethodVisitor visitMethod(final int access, final String mname, final String desc,
						final String sig, final String[] ex) {
					return new Rewriter(super.visitMethod(access, mname, desc, sig, ex));
				}
			}, 0);
			if (sites.get() == before) {
				return null;
			}
			classes.incrementAndGet();
			return cw.toByteArray();
		} catch (final Throwable t) {
			System.err.println("[seed-agent] could not rewrite " + name + ": " + t);
			return null;
		}
	}

	static String swap(final String owner) {
		return switch (owner) {
		case "java/util/HashSet" -> "java/util/LinkedHashSet";
		case "java/util/HashMap" -> "java/util/LinkedHashMap";
		default -> null;
		};
	}

	private final class Rewriter extends MethodVisitor {
		/** Rewritten NEWs whose constructor call is still pending (super() calls are never rewritten). */
		private int pending;

		Rewriter(final MethodVisitor mv) {
			super(Opcodes.ASM9, mv);
		}

		@Override
		public void visitTypeInsn(final int opcode, final String type) {
			if (opcode == Opcodes.NEW && swap(type) != null) {
				pending++;
				sites.incrementAndGet();
				super.visitTypeInsn(opcode, swap(type));
				return;
			}
			super.visitTypeInsn(opcode, type);
		}

		@Override
		public void visitMethodInsn(final int opcode, final String owner, final String name, final String desc,
				final boolean itf) {
			if (opcode == Opcodes.INVOKESPECIAL && "<init>".equals(name) && swap(owner) != null && pending > 0) {
				pending--;
				super.visitMethodInsn(opcode, swap(owner), name, desc, itf);
				return;
			}
			if (opcode == Opcodes.INVOKESTATIC) {
				if ("java/util/stream/Collectors".equals(owner)
						&& ("toSet".equals(name) || "toUnmodifiableSet".equals(name))) {
					sites.incrementAndGet();
					super.visitMethodInsn(opcode, ORDERED, name, desc, false);
					return;
				}
				if ("java/util/Set".equals(owner) && "of".equals(name)) {
					sites.incrementAndGet();
					final boolean array = desc.startsWith("([Ljava/lang/Object;)");
					super.visitMethodInsn(opcode, ORDERED, array ? "setOfArray" : "setOf", desc, false);
					return;
				}
				if ("java/util/Set".equals(owner) && "copyOf".equals(name)) {
					sites.incrementAndGet();
					super.visitMethodInsn(opcode, ORDERED, "setCopyOf", desc, false);
					return;
				}
			}
			super.visitMethodInsn(opcode, owner, name, desc, itf);
		}

		@Override
		public void visitInvokeDynamicInsn(final String name, final String desc, final Handle bsm,
				final Object... args) {
			for (int i = 0; i < args.length; i++) {
				if (args[i] instanceof final Handle h && h.getTag() == Opcodes.H_NEWINVOKESPECIAL
						&& swap(h.getOwner()) != null) {
					args[i] = new Handle(h.getTag(), swap(h.getOwner()), h.getName(), h.getDesc(), h.isInterface());
					sites.incrementAndGet();
				} else if (args[i] instanceof final Handle h && h.getTag() == Opcodes.H_INVOKESTATIC
						&& "java/util/Set".equals(h.getOwner()) && "of".equals(h.getName())) {
					args[i] = new Handle(h.getTag(), ORDERED,
							h.getDesc().startsWith("([Ljava/lang/Object;)") ? "setOfArray" : "setOf", h.getDesc(),
							false);
					sites.incrementAndGet();
				}
			}
			super.visitInvokeDynamicInsn(name, desc, bsm, args);
		}
	}
}
