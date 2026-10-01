package refsim.trace;

import org.eclipse.emf.ecore.EObject;
import org.palladiosimulator.pcm.core.composition.AssemblyContext;
import org.palladiosimulator.simulizar.interpreter.listener.AssemblyProvidedOperationPassedEvent;
import org.palladiosimulator.simulizar.interpreter.listener.EventType;
import org.palladiosimulator.simulizar.interpreter.listener.ModelElementPassedEvent;
import org.palladiosimulator.simulizar.interpreter.listener.RDSEFFElementPassedEvent;
import org.palladiosimulator.simulizar.interpreter.listener.SystemOperationPassedEvent;

/** Trace helpers that need SimuLizar types (called from patched SimuLizar classes only). */
public final class SimulizarHooks {
    private SimulizarHooks() {
    }

    /** Every interpreter "passed" event: usage scenario, user actions, SEFF actions, operation calls. */
    public static void passed(ModelElementPassedEvent<? extends EObject> event) {
        String ev = event.getEventType() == EventType.BEGIN ? "begin" : "end";
        Object p = event.getThread();
        if (event instanceof SystemOperationPassedEvent) {
            SystemOperationPassedEvent<?, ?, ?> e = (SystemOperationPassedEvent<?, ?, ?>) event;
            Trace.event(ev, p, "type", "SystemOperation", "role", Trace.id(e.getProvidedRole()), "sig",
                    Trace.id(e.getSignature()));
        } else if (event instanceof AssemblyProvidedOperationPassedEvent) {
            AssemblyProvidedOperationPassedEvent<?, ?, ?> e = (AssemblyProvidedOperationPassedEvent<?, ?, ?>) event;
            Trace.event(ev, p, "type", "AssemblyOperation", "ac", Trace.id(e.getAssemblyContext()), "role",
                    Trace.id(e.getProvidedRole()), "sig", Trace.id(e.getSignature()));
        } else {
            EObject el = event.getModelElement();
            String ac = null;
            if (event instanceof RDSEFFElementPassedEvent) {
                AssemblyContext a = ((RDSEFFElementPassedEvent<?>) event).getAssemblyContext();
                ac = Trace.id(a);
            }
            Trace.element(ev, p, el.eClass().getName(), Trace.id(el), ac);
        }
    }
}
