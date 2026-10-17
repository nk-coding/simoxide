package simoxide.slingshot.seed;

import java.lang.instrument.ClassFileTransformer;
import java.security.ProtectionDomain;
import java.util.Map;

import org.objectweb.asm.ClassReader;
import org.objectweb.asm.ClassVisitor;
import org.objectweb.asm.ClassWriter;
import org.objectweb.asm.MethodVisitor;
import org.objectweb.asm.Opcodes;

/**
 * Inserts {@code Sites.set(<local>)} at the entry of the methods that draw from {@code Math.random()}, so
 * that the tape can name the draw site (see {@link Sites}). Instrumentation only: the inserted call stores a
 * reference and returns; no Slingshot state, order or draw changes. Installed whenever the agent is loaded
 * (det and seeded stock runs).
 */
final class SiteTransformer implements ClassFileTransformer {

	/** class -> method name -> local variable slot to announce (0 = this, 1 = first argument). */
	static final Map<String, Map<String, Integer>> HOOKS = Map.of(
			"org/palladiosimulator/analyzer/slingshot/common/utils/TransitionDeterminer",
			Map.of("determineBranchTransition", 1, "determineProbabilisticBranchTransition", 1),
			"org/palladiosimulator/analyzer/slingshot/behavior/resourcesimulation/resources/linking/SimulatedLinkingResource",
			Map.of("onJobProgressed", 0));

	volatile int hooked;

	@Override
	public byte[] transform(final Module module, final ClassLoader loader, final String name,
			final Class<?> redefined, final ProtectionDomain pd, final byte[] bytes) {
		final Map<String, Integer> methods = name == null ? null : HOOKS.get(name);
		if (methods == null) {
			return null;
		}
		try {
			final ClassReader cr = new ClassReader(bytes);
			final ClassWriter cw = new ClassWriter(cr, ClassWriter.COMPUTE_MAXS);
			cr.accept(new ClassVisitor(Opcodes.ASM9, cw) {
				@Override
				public MethodVisitor visitMethod(final int access, final String mname, final String desc,
						final String sig, final String[] ex) {
					final MethodVisitor mv = super.visitMethod(access, mname, desc, sig, ex);
					final Integer slot = methods.get(mname);
					if (slot == null || (access & Opcodes.ACC_BRIDGE) != 0 || (access & Opcodes.ACC_SYNTHETIC) != 0) {
						return mv;
					}
					return new MethodVisitor(Opcodes.ASM9, mv) {
						@Override
						public void visitCode() {
							super.visitCode();
							super.visitVarInsn(Opcodes.ALOAD, slot);
							super.visitMethodInsn(Opcodes.INVOKESTATIC, "simoxide/slingshot/seed/Sites", "set",
									"(Ljava/lang/Object;)V", false);
							hooked++;
						}
					};
				}
			}, 0);
			return cw.toByteArray();
		} catch (final Throwable t) {
			System.err.println("[seed-agent] could not hook draw sites in " + name + ": " + t);
			return null;
		}
	}
}
